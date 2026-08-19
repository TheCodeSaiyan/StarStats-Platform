//! Resumable `Game.log` tailer.
//!
//! Two layers:
//! - `notify::RecommendedWatcher` fires on every filesystem change to
//!   the file. We translate that into a "drain pending bytes" signal.
//! - A `tokio` task seeks to the saved byte offset, reads complete
//!   lines, parses them via `starstats-core`, and stores recognised
//!   events in SQLite.
//!
//! Rotation handling: at game launch the log is replaced. We detect
//! this by a change in the file's head signature (`file_signature`),
//! not just `metadata.len() < offset` — a fresh log that has already
//! grown past our saved offset has the same length relationship as an
//! in-place append, so the length check alone would seek mid-file and
//! skip the new session's opening lines. See `resolve_resume_offset`.

use crate::burst_rules::builtin_burst_rules;
use crate::parser_defs::RuleCache;
use crate::storage::Storage;
use anyhow::Result;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use starstats_core::templates::{
    build_loadout_categories, build_loadout_items, detect_bursts, BurstRule,
};
use starstats_core::unknown_lines::CaptureContextOwned;
use starstats_core::wire::{EventEnvelope, LogSource};
use starstats_core::{
    apply_remote_rules, classify, classify_or_capture, infer_with_rules, structural_parse,
    BurstSummary, ClassifyOutcome, CompiledInferenceRule, GameEvent, InferenceConfig, LogLine,
};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader, SeekFrom};
use tokio::sync::{mpsc, Notify};

/// How many classified envelopes the rolling inference window keeps
/// in memory across drains. Sized for the longest cross-drain rule
/// (`implicit_shop_request_timeout` at 30s, conservatively bounded by
/// event count) — well below the 200-event `InferenceConfig::window_size`
/// so the per-pass cost stays linear and bounded.
const INFERENCE_WINDOW_CAPACITY: usize = 50;

/// Rolling state that lets the inference pass span drain boundaries.
/// One instance lives in each ingest task (live tail or backfill file
/// loop) and is mutated in place as new observed envelopes land.
///
/// `recent` caps at [`INFERENCE_WINDOW_CAPACITY`] — once full, every
/// push drops the oldest entry so the slice stays bounded.
/// `emitted` is the idempotency_keys of inferred envelopes the task has
/// already persisted, so a sliding window doesn't double-emit when an
/// earlier trigger fires the same rule again on a later pass.
#[derive(Default)]
pub(crate) struct InferenceWindow {
    recent: VecDeque<EventEnvelope>,
    emitted: HashSet<String>,
}

impl InferenceWindow {
    fn push(&mut self, envelope: EventEnvelope) {
        if self.recent.len() == INFERENCE_WINDOW_CAPACITY {
            self.recent.pop_front();
        }
        self.recent.push_back(envelope);
    }

    fn as_slice(&self) -> Vec<EventEnvelope> {
        self.recent.iter().cloned().collect()
    }
}

/// Live counters surfaced to the frontend.
#[derive(Debug, Default, Clone, Serialize)]
pub struct TailStats {
    pub current_path: Option<PathBuf>,
    pub bytes_read: u64,
    pub lines_processed: u64,
    pub events_recognised: u64,
    pub last_event_at: Option<String>,
    pub last_event_type: Option<String>,
    /// Lines that produced a `LogLine` (timestamp + body) but for
    /// which `classify` returned `None`. These are the actionable
    /// "we should write a parser rule for this" cases.
    pub lines_structural_only: u64,
    /// Lines the structural parser couldn't handle at all — banners,
    /// blanks, continuation lines, etc. Not actionable as parser rules.
    pub lines_skipped: u64,
    /// Lines whose event_name was on the noise list — we recognised
    /// them as engine-internal chatter and dropped them on purpose.
    /// Counted separately so the user can see "we filtered N noise
    /// lines" rather than silently hiding them.
    pub lines_noise: u64,
}

/// Start watching `path` and tailing its appended bytes. Returns the
/// watcher handle — drop it to stop watching.
///
/// `enable_v2_metadata` gates the v2 event-handling pipeline (unknown
/// line capture into the local SQLite review queue). When false, the
/// legacy `record_unknown` sample path remains the sole surface for
/// unclassified lines so a flag-off install behaves exactly like the
/// pre-Phase-3 build.
///
/// `event_kick` is fired (`notify_one()`) after any drain that ingested
/// new events, so downstream consumers waiting on it — the opt-in
/// org-platform connector — forward presence the instant it lands
/// instead of polling. Consumers that don't care simply never await it.
pub async fn start_tail(
    path: PathBuf,
    storage: Arc<Storage>,
    stats: Arc<parking_lot::Mutex<TailStats>>,
    rules: RuleCache,
    enable_v2_metadata: bool,
    own_handle: String,
    event_kick: Arc<Notify>,
) -> Result<RecommendedWatcher> {
    let (tx, mut rx) = mpsc::channel::<()>(64);

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if matches!(
                ev.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
            ) {
                let _ = tx.try_send(());
            }
        }
    })?;

    watcher.watch(&path, RecursiveMode::NonRecursive)?;

    let path_str = path.to_string_lossy().to_string();
    // Load the offset together with the signature of the file it
    // belongs to, so the first drain after a tray restart can detect a
    // rotation that happened while we were down.
    let (mut offset, mut last_sig) = storage.read_tail_cursor(&path_str)?;

    let path_clone = path.clone();
    let path_str_clone = path_str.clone();
    let storage_clone = Arc::clone(&storage);
    let stats_clone = Arc::clone(&stats);

    let rules_clone = rules.clone();
    tokio::spawn(async move {
        // One inference window per tail task — survives across drains so
        // a trigger in drain N can pair with a follow-up in drain N+1.
        let mut window = InferenceWindow::default();
        // Initial drain in case the file already has new data we haven't seen.
        match drain(
            &path_clone,
            &path_str_clone,
            &mut offset,
            &mut last_sig,
            &storage_clone,
            &stats_clone,
            &rules_clone,
            enable_v2_metadata,
            &own_handle,
            &mut window,
        )
        .await
        {
            // A drain that ingested lines wakes downstream consumers so
            // presence is forwarded immediately, not on their next poll.
            Ok(true) => event_kick.notify_one(),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "initial tail drain failed"),
        }

        while rx.recv().await.is_some() {
            // Coalesce bursts of filesystem events.
            while rx.try_recv().is_ok() {}
            match drain(
                &path_clone,
                &path_str_clone,
                &mut offset,
                &mut last_sig,
                &storage_clone,
                &stats_clone,
                &rules_clone,
                enable_v2_metadata,
                &own_handle,
                &mut window,
            )
            .await
            {
                Ok(true) => event_kick.notify_one(),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "tail drain failed; backing off");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });

    Ok(watcher)
}

/// Returns `Ok(true)` when this drain ingested at least one new line
/// (and therefore may have written events), `Ok(false)` when there was
/// nothing new. Callers use the flag to wake downstream consumers only
/// when there's actually fresh data to forward.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn drain(
    path: &PathBuf,
    path_str: &str,
    offset: &mut u64,
    last_sig: &mut Option<String>,
    storage: &Storage,
    stats: &parking_lot::Mutex<TailStats>,
    rules: &RuleCache,
    enable_v2_metadata: bool,
    own_handle: &str,
    window: &mut InferenceWindow,
) -> Result<bool> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    let metadata = file.metadata().await?;

    // Reconcile against the file we last read. A changed signature means
    // the launcher rotated in a fresh log (even one already longer than
    // our saved offset — the case the old `len < offset` check missed);
    // a shorter file means in-place truncation. Either way, resume at
    // the head. Runs BEFORE the `len == offset` short-circuit so a
    // same-length replacement is also caught.
    let current_sig = file_signature(&mut file, &metadata).await?;
    let resumed = resolve_resume_offset(
        *offset,
        last_sig.as_deref(),
        current_sig.as_deref(),
        metadata.len(),
    );
    if resumed != *offset {
        tracing::info!(
            previous = *offset,
            resumed,
            current_len = metadata.len(),
            rotated = current_sig != *last_sig,
            "tail resume offset reset (rotation/truncation)"
        );
    }
    *offset = resumed;
    *last_sig = current_sig;

    if metadata.len() == *offset {
        // Nothing new.
        return Ok(false);
    }

    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(*offset)).await?;
    let mut buf = String::new();

    let log_source = log_source_from_path(path);
    let log_source_enum = log_source_enum_from_str(&log_source);

    let rules_snapshot = rules.snapshot();
    let inference_rules = rules.combined_inference_rules();
    let burst_rules = builtin_burst_rules();

    // Accumulate this drain's complete lines into a buffer so the
    // burst matcher can see the run as a unit. Per-line ingest can't
    // detect a burst because the first AttachmentReceived is already
    // committed to storage by the time the second one arrives;
    // batching at drain boundaries solves that without needing a
    // cross-drain rolling buffer (live tails fire on every filesystem
    // notify, so a burst that lands in one fsync arrives in one
    // drain in practice).
    let mut buffered_lines: Vec<(String, u64)> = Vec::new();
    loop {
        let line_start = *offset;
        buf.clear();
        let n = reader.read_line(&mut buf).await?;
        if n == 0 {
            break;
        }
        if !buf.ends_with('\n') {
            // Partial line — leave for next drain.
            break;
        }
        *offset += n as u64;

        let trimmed = buf.trim_end_matches(['\r', '\n']).to_string();
        buffered_lines.push((trimmed, line_start));
    }

    let processed = !buffered_lines.is_empty();
    if processed {
        // `process_buffer` stitches multi-line HUD notification records
        // itself (shared with backfill — see its doc comment below), so
        // `buffered_lines` here is still one entry per PHYSICAL line and
        // `lines_processed` stays a read counter, not a record counter.
        process_buffer(
            &buffered_lines,
            storage,
            stats,
            &log_source,
            log_source_enum,
            last_sig.as_deref(),
            &rules_snapshot,
            &inference_rules,
            &burst_rules,
            enable_v2_metadata,
            own_handle,
            window,
        );
        let mut s = stats.lock();
        s.bytes_read = *offset;
        s.lines_processed += buffered_lines.len() as u64;
    }

    storage.write_tail_cursor(path_str, *offset, last_sig.as_deref())?;
    Ok(processed)
}

/// Maximum physical lines one logical record may span. Real logs show at
/// most three (9 of 238 joins in a 40-log corpus); the cap stops a
/// malformed or truncated line from consuming the whole drain buffer.
const MAX_STITCH_LINES: usize = 3;

/// Rejoin HUD-notification records the game wrote across multiple physical
/// lines.
///
/// The game splits some notifications mid-string, so the closing quote,
/// `MissionId` and `ObjectiveId` land on a continuation line. A
/// line-oriented parser sees a truncated first line with no closing quote,
/// fails to classify it, and drops the record into `unknown_lines` — which
/// is why `Objective Complete` banners were invisible.
///
/// The join predicate is deliberately NARROW: a line is only a stitch
/// candidate when it contains `Added notification "` AND has an odd number
/// of quotes (an unterminated string). A general "join lines that don't
/// look like a new record" heuristic was tried against real logs and
/// collapsed 128,031 lines into 22,118 — it swallowed ~106,000 unrelated
/// lines. Everything that is not an unterminated HUD notification passes
/// through byte-identical. As a second line of defence, a candidate never
/// absorbs a line that itself looks like a well-formed record start
/// (`<ts> [...`) — quote-parity alone can't tell "unterminated" from "a
/// complete record with an odd number of embedded quotes", and without
/// this guard the latter would swallow a real following record.
///
/// The merged record keeps the FIRST line's byte offset. `source_offset`
/// is where a resumed tail seeks, so using the continuation's offset would
/// re-emit or skip records across a restart. The join separator is a
/// single space, NOT the `\n` the game's log actually has at the split.
/// Two things force this: (1) fusing the halves together with nothing
/// between them mashes the last token of the first half into the first
/// token of the continuation (`"...above Euterpe: \" [15]"` becomes
/// `"...above EuterpePyriel138"`-shaped garbage for a name-adjacent split);
/// (2) a *literal* `\n` is not just cosmetic — `SHELL_RE`'s trailing
/// `(?P<rest>.*)$` cannot span an embedded newline (the `regex` crate
/// leaves `dot_matches_new_line` off by default and `$` anchors to
/// end-of-haystack, not end-of-line), so a `\n`-joined record fails
/// `structural_parse` entirely and is silently dropped as
/// `IngestOutcome::Skipped` before it ever reaches classification or
/// `record_unknown`. A future "restore byte fidelity" pass that swaps this
/// back to `\n` would silently zero out every recovered record; see
/// `stitched_hud_notification_reaches_storage_via_process_buffer` for the
/// regression test that guards this. A single space fixes the token-mash
/// without breaking the parse.
///
/// Only called from [`process_buffer`], so a record whose halves land in
/// two different `process_buffer` calls — a live-tail drain boundary, or a
/// backfill chunk boundary past the carry hold-back — is not stitched. Not
/// a regression: before this feature such a record was invisible either way.
fn stitch_multiline_records(mut lines: Vec<(String, u64)>) -> Vec<(String, u64)> {
    fn unterminated(s: &str) -> bool {
        s.contains("Added notification \"") && s.matches('"').count() % 2 == 1
    }
    // Strip a leading `<ts> ` from a continuation — the timestamp is
    // redundant (the merged record already carries the first line's), and
    // dropping it here means the join produces exactly `first line text +
    // ' ' + this` for each continuation rather than a reconstruction that
    // duplicates the continuation's own timestamp.
    fn continuation_body(s: &str) -> &str {
        s.strip_prefix('<')
            .and_then(|rest| rest.split_once("> "))
            .map(|(_, body)| body)
            .unwrap_or(s)
    }
    // A *complete* notification can still carry an odd number of quotes
    // (an apostrophe-adjacent name, a stray literal quote in the banner
    // text). Refuse to treat the following line as a continuation when it
    // looks like a real record start itself, so a mis-detected candidate
    // can't swallow it. Measured on the 280-log corpus: 0 of 2,943
    // absorbed lines look like this today — theoretical hardening.
    fn looks_like_record_start(s: &str) -> bool {
        s.strip_prefix('<')
            .and_then(|rest| rest.split_once('>'))
            .is_some_and(|(_, rest)| rest.starts_with(" ["))
    }

    let mut out: Vec<(String, u64)> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        if !unterminated(&lines[i].0) {
            // Move rather than clone — this is the ~99.92% pass-through
            // case, so it shouldn't pay for a copy of every line.
            let (text, offset) = std::mem::take(&mut lines[i]);
            out.push((text, offset));
            i += 1;
            continue;
        }

        // Dry-run the join using only references first, so we commit
        // (via `mem::take`) to exactly one outcome instead of cloning the
        // first line once for a tentative merge and again for a possible
        // fallback.
        let mut quote_count = lines[i].0.matches('"').count();
        let mut consumed = 1usize;
        let mut closed = quote_count % 2 == 0;
        while !closed && consumed < MAX_STITCH_LINES && i + consumed < lines.len() {
            let next = &lines[i + consumed].0;
            if looks_like_record_start(next) {
                break;
            }
            quote_count += continuation_body(next).matches('"').count();
            consumed += 1;
            closed = quote_count % 2 == 0;
        }

        let offset = lines[i].1;
        if closed {
            // Closed: emit the rejoined record at the first line's offset.
            // Separator MUST be something SHELL_RE's trailing `.*$` can
            // span — see the doc comment above. A literal `\n` looks more
            // "byte-faithful" but silently kills the parse; don't restore
            // it.
            let mut merged = std::mem::take(&mut lines[i].0);
            for k in 1..consumed {
                merged.push(' ');
                merged.push_str(continuation_body(&lines[i + k].0));
            }
            out.push((merged, offset));
            i += consumed;
        } else {
            // Never closed within the cap (or blocked by a following
            // record-start line) — leave the original line alone. It
            // fails to classify exactly as it does today (no regression),
            // and the continuation, if it ever arrives, is handled on its
            // own.
            out.push((std::mem::take(&mut lines[i].0), offset));
            i += 1;
        }
    }
    out
}

/// Process a batch of lines from one drain. Four passes:
///   0. Stitch multi-line HUD notification records back together (see
///      [`stitch_multiline_records`]) — the ONE call site shared by both
///      the live tail (`drain`) and backfill (`backfill_file`), so
///      historical `logbackups/*.log` archives get the same recovered
///      `Objective Complete` records as freshly-tailed lines. Known
///      limitation: backfill replays a rotated log in
///      `BACKFILL_CHUNK_LINES`-sized chunks, so a record straddling a
///      chunk boundary still won't stitch — the existing burst carry
///      hold-back doesn't apply to this. Not a regression: today such a
///      record is invisible either way.
///   1. Structural-parse every line into a `LogLine` (or `None`).
///   2. Run [`detect_bursts`] over the parseable subset and translate
///      member indices back to buffer positions; insert one
///      [`BurstSummary`] per hit.
///   3. Per-line classify+ingest for every line that wasn't claimed
///      by a burst.
///
/// Bursts dedupe against re-drains via an offset-based idempotency
/// key (the anchor line offset + the rule id + size), so a tray
/// crash mid-flush re-emits the same summary on retry rather than
/// producing duplicates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn process_buffer(
    buffer: &[(String, u64)],
    storage: &Storage,
    stats: &parking_lot::Mutex<TailStats>,
    log_source: &str,
    log_source_enum: LogSource,
    file_sig: Option<&str>,
    remote_rules: &[starstats_core::CompiledRemoteRule],
    inference_rules: &[CompiledInferenceRule],
    burst_rules: &[BurstRule],
    enable_v2_metadata: bool,
    own_handle: &str,
    window: &mut InferenceWindow,
) {
    // Pass 0: see the doc comment above. Both call sites (live tail and
    // backfill) get the fix for free from this one spot.
    let stitched = stitch_multiline_records(buffer.to_vec());
    let buffer: &[(String, u64)] = &stitched;

    // Pass 1: parse every line. The Vec<Option<LogLine>> preserves
    // index alignment with `buffer` so we can map burst indices back
    // to buffer positions in pass 2.
    let parsed: Vec<Option<LogLine<'_>>> = buffer
        .iter()
        .map(|(line, _)| structural_parse(line))
        .collect();

    // Project to the parseable subset (with their original buffer
    // indices) so the matcher sees a contiguous slice of LogLines.
    let valid: Vec<(usize, LogLine<'_>)> = parsed
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.as_ref().map(|line| (i, line.clone())))
        .collect();
    let valid_lines: Vec<LogLine<'_>> = valid.iter().map(|(_, l)| l.clone()).collect();

    // Pass 2: burst detection. Indices are into `valid_lines`.
    let bursts = detect_bursts(&valid_lines, burst_rules);

    // Map burst-member indices back to original buffer indices for
    // suppression in pass 3.
    let mut suppressed: HashSet<usize> = HashSet::new();
    for burst in &bursts {
        for &valid_idx in &burst.member_indices {
            suppressed.insert(valid[valid_idx].0);
        }
    }

    // Pass 2b: emit one BurstSummary per burst.
    for burst in &bursts {
        let anchor_buf_idx = valid[burst.start_index].0;
        let end_buf_idx = valid[burst.end_index].0;
        let anchor_log = &valid_lines[burst.start_index];
        let end_log = &valid_lines[burst.end_index];
        let anchor_line = &buffer[anchor_buf_idx].0;
        let anchor_offset = buffer[anchor_buf_idx].1;
        // The anchor body can be huge (full inventory dump); cap
        // before storing so the timeline doesn't carry kilobytes per
        // burst summary.
        let sample: String = anchor_log.body.chars().take(200).collect();

        // For loadout_restore bursts, classify each member line to
        // extract its item_class, then build the category-count map
        // the web loadout widget needs. NOTE: the rule id is
        // "loadout_restore_burst" (see burst_rules.rs) — matching the
        // bare "loadout_restore" here was a silent bug that left every
        // loadout burst with kind=None, so the widget never rendered it.
        let (burst_kind, burst_categories, burst_items) = if burst.rule_id
            == crate::burst_rules::LOADOUT_RESTORE_BURST_RULE_ID
        {
            let pairs: Vec<(String, String)> = burst
                .member_indices
                .iter()
                .filter_map(|&vi| {
                    let line = &valid_lines[vi];
                    if let Some(GameEvent::AttachmentReceived(ar)) = classify(line) {
                        Some((ar.item_class, ar.port))
                    } else {
                        None
                    }
                })
                .collect();
            let categories =
                build_loadout_categories(&pairs.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>());
            let items = build_loadout_items(&pairs);
            (Some("loadout_restore".to_string()), categories, items)
        } else {
            (None, None, None)
        };

        let summary = GameEvent::BurstSummary(BurstSummary {
            timestamp: anchor_log.timestamp.to_string(),
            rule_id: burst.rule_id.clone(),
            size: burst.size as u32,
            end_timestamp: end_log.timestamp.to_string(),
            anchor_body_sample: if sample.is_empty() {
                None
            } else {
                Some(sample)
            },
            kind: burst_kind,
            categories: burst_categories,
            items: burst_items,
        });

        let Some((event_type, ts, payload)) = serialise_event(&summary) else {
            tracing::warn!(rule = %burst.rule_id, "burst summary failed to serialise");
            continue;
        };
        // Synthetic key: anchor offset + rule id + size. Same
        // (offset, rule, size) on retry produces the same key, so
        // re-drains after a crash dedupe via the UNIQUE constraint.
        // Anchor offset alone isn't enough — two rules could both
        // anchor on the same offset (rare; first-rule-wins handles
        // most cases, but the key is defensive).
        let synthetic_line = format!("{anchor_line}|burst:{}:{}", burst.rule_id, burst.size);
        let key = idempotency_key(log_source, file_sig, anchor_offset, &synthetic_line);
        // Idempotency already covers retries; if the insert fails for
        // another reason, log and continue — better one missing summary
        // than no events at all.
        if let Err(e) = storage.insert_event(
            &key,
            &event_type,
            &ts,
            anchor_line,
            &payload,
            log_source,
            anchor_offset,
        ) {
            tracing::warn!(error = %e, rule = %burst.rule_id, "insert burst summary failed");
            continue;
        }
        if enable_v2_metadata {
            window.push(EventEnvelope {
                idempotency_key: key.clone(),
                raw_line: anchor_line.clone(),
                event: Some(summary.clone()),
                source: log_source_enum,
                source_offset: anchor_offset,
                metadata: None,
                // Local recent-events window — feeds inference, not the
                // sync wire. Resolution is stamped at sync time in
                // `sync::build_batch`, so this path carries None.
                resolved_location: None,
            });
        }
        let _ = end_buf_idx; // captured for future end-marker rendering
        let mut s = stats.lock();
        s.events_recognised += 1;
        s.last_event_type = Some(event_type);
        s.last_event_at = Some(ts);
    }

    // Pass 3: per-line ingest for everything not consumed by a burst.
    for (i, (line, line_offset)) in buffer.iter().enumerate() {
        if suppressed.contains(&i) {
            continue;
        }
        process_line(
            line,
            storage,
            stats,
            log_source,
            log_source_enum,
            *line_offset,
            file_sig,
            remote_rules,
            enable_v2_metadata,
            own_handle,
            window,
        );
    }

    // Pass 4: run the inference engine over the rolling window. Only
    // fires when the v2 pipeline is enabled — the gate keeps a
    // flag-off install on the legacy timeline shape exactly.
    if enable_v2_metadata {
        run_inference(storage, stats, log_source, inference_rules, window);
    }
}

/// Max trailing lines a backfill chunk holds back so a burst straddling
/// the 10k-line chunk boundary can be re-detected whole with the next
/// chunk. Generous — real attachment/loadout bursts are dozens of lines.
pub(crate) const BURST_CARRY_LINES: usize = 256;

/// Compute the commit/carry split for a backfill chunk buffer: the caller
/// processes `buffer[..cut]` now and prepends `buffer[cut..]` to the next
/// chunk. Guarantees **no detected burst spans `cut`** — a burst reaching
/// the trailing `max_carry` lines (so it might continue in the next chunk)
/// is carried WHOLE rather than emitted as a bogus partial. Pure given the
/// bursts' `(anchor, end)` BUFFER-index ranges, so it's exhaustively unit
/// -testable.
fn carry_boundary(len: usize, burst_ranges: &[(usize, usize)], max_carry: usize) -> usize {
    if len <= max_carry {
        // Small buffer: carry it all and wait for more before committing.
        return 0;
    }
    let mut cut = len - max_carry;
    for &(anchor, end) in burst_ranges {
        // A burst reaching into the carried tail might continue in the
        // next chunk → carry the whole burst (never commit a partial).
        if end >= cut {
            cut = cut.min(anchor);
        }
    }
    cut
}

/// Map detected bursts in `buffer` to their `(anchor, end)` BUFFER-index
/// ranges — mirrors [`process_buffer`] passes 1-2 (parse → `detect_bursts`
/// → valid-index-to-buffer-index remap).
fn burst_buffer_ranges(buffer: &[(String, u64)], burst_rules: &[BurstRule]) -> Vec<(usize, usize)> {
    let parsed: Vec<Option<LogLine<'_>>> = buffer
        .iter()
        .map(|(line, _)| structural_parse(line))
        .collect();
    let valid: Vec<(usize, LogLine<'_>)> = parsed
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.as_ref().map(|line| (i, line.clone())))
        .collect();
    let valid_lines: Vec<LogLine<'_>> = valid.iter().map(|(_, l)| l.clone()).collect();
    detect_bursts(&valid_lines, burst_rules)
        .iter()
        .map(|b| (valid[b.start_index].0, valid[b.end_index].0))
        .collect()
}

/// The commit/carry split for a backfill chunk buffer (see
/// [`carry_boundary`]). `pub(crate)` so the backfill chunk loop can hold
/// back a straddling burst's tail; `0` means "carry everything, no commit
/// yet" (buffer smaller than the carry window).
pub(crate) fn backfill_carry_split(buffer: &[(String, u64)], burst_rules: &[BurstRule]) -> usize {
    let ranges = burst_buffer_ranges(buffer, burst_rules);
    carry_boundary(buffer.len(), &ranges, BURST_CARRY_LINES)
}

/// Run inference over the rolling window and persist any new
/// inferred events. The synthetic envelope's `idempotency_key` is
/// deterministic over `(rule_id, trigger_idempotency_key)` so a
/// rule firing twice for the same trigger (e.g. the window slides
/// forward and the trigger is still in `recent`) collapses to one
/// row via both `emitted` (in-memory) and `ON CONFLICT DO NOTHING`
/// (storage) — defence in depth against double-emission.
///
/// Inferred rows are persisted through the same `insert_event`
/// path as observed rows, so the sync worker ships them to the
/// server alongside everything else. `EventMetadata` for inferred
/// fields isn't persisted on the client today (see follow-up
/// "`metadata::stamp` is server-side only"); the synthetic
/// envelope's `raw_line` carries the rule_id so the server can
/// recognise the row as inferred even without metadata.
///
/// Supersede handling (Phase 3 design §inference): out of scope
/// for this wiring task. When an observed event later supersedes
/// an inferred row of the same `(event_type, primary_entity)`,
/// timeline consumers can elect to drop the inferred row at render
/// time; storage retains both.
fn run_inference(
    storage: &Storage,
    stats: &parking_lot::Mutex<TailStats>,
    log_source: &str,
    inference_rules: &[CompiledInferenceRule],
    window: &mut InferenceWindow,
) {
    let inserted = run_inference_and_persist(storage, log_source, inference_rules, window);
    for (event_type, timestamp) in inserted {
        let mut s = stats.lock();
        s.events_recognised += 1;
        s.last_event_type = Some(event_type);
        s.last_event_at = Some(timestamp);
    }
}

/// Core inference persistence used by [`run_inference`]. Returns the
/// `(event_type, timestamp)` of each persisted inferred row so the
/// caller can update its "last event" stats. Backfill reaches this same
/// path indirectly by replaying through [`process_buffer`].
fn run_inference_and_persist(
    storage: &Storage,
    log_source: &str,
    inference_rules: &[CompiledInferenceRule],
    window: &mut InferenceWindow,
) -> Vec<(String, String)> {
    if inference_rules.is_empty() {
        return Vec::new();
    }
    let envelopes = window.as_slice();
    if envelopes.is_empty() {
        return Vec::new();
    }
    let config = InferenceConfig::default();
    let inferred = infer_with_rules(&envelopes, &config, inference_rules);
    let mut persisted = Vec::new();
    for row in inferred {
        let key = inferred_idempotency_key(
            row.metadata.rule_id.as_deref().unwrap_or(""),
            &row.trigger_idempotency_key,
        );
        if !window.emitted.insert(key.clone()) {
            continue;
        }
        let Some((event_type, timestamp, payload)) = serialise_event(&row.event) else {
            tracing::warn!(rule_id = ?row.metadata.rule_id, "inferred event failed to serialise");
            continue;
        };
        let raw_line = format!(
            "inferred:{}",
            row.metadata.rule_id.as_deref().unwrap_or("unknown")
        );
        if let Err(e) = storage.insert_event(
            &key,
            &event_type,
            &timestamp,
            &raw_line,
            &payload,
            log_source,
            0, // synthetic — inferred events have no source byte offset
        ) {
            tracing::warn!(error = %e, rule_id = ?row.metadata.rule_id, "insert inferred event failed");
            continue;
        }
        persisted.push((event_type, timestamp));
    }
    persisted
}

#[allow(clippy::too_many_arguments)]
fn process_line(
    line: &str,
    storage: &Storage,
    stats: &parking_lot::Mutex<TailStats>,
    log_source: &str,
    log_source_enum: LogSource,
    line_offset: u64,
    file_sig: Option<&str>,
    rules: &[starstats_core::CompiledRemoteRule],
    enable_v2_metadata: bool,
    own_handle: &str,
    window: &mut InferenceWindow,
) {
    match ingest_one_line(
        line,
        storage,
        log_source,
        log_source_enum,
        line_offset,
        file_sig,
        rules,
        enable_v2_metadata,
        own_handle,
        Some(window),
    ) {
        IngestOutcome::Skipped => {
            stats.lock().lines_skipped += 1;
        }
        IngestOutcome::Noise => {
            stats.lock().lines_noise += 1;
        }
        IngestOutcome::StructuralOnly => {
            stats.lock().lines_structural_only += 1;
        }
        IngestOutcome::Recognised {
            event_type,
            timestamp,
        } => {
            let mut s = stats.lock();
            s.events_recognised += 1;
            s.last_event_type = Some(event_type);
            s.last_event_at = Some(timestamp);
        }
    }
}

/// What happened to a single line during ingest. Surfaced so the
/// caller can update its own stats — `process_line` (live tail) and
/// the backfill module both wrap this with their own counter shape.
#[derive(Debug)]
pub(crate) enum IngestOutcome {
    /// Structural parse failed — banner, blank, continuation line.
    Skipped,
    /// Structural parse OK; event_name was on the user's noise list.
    Noise,
    /// Structural parse OK; classifier didn't recognise the event_name
    /// (or it had no event_name). A sample is recorded in the
    /// `unknowns` table for surface area.
    StructuralOnly,
    /// Event classified, serialised, inserted (or deduped via the
    /// idempotency key — both paths return this).
    Recognised {
        event_type: String,
        timestamp: String,
    },
}

/// Stats-free ingest of one log line. The caller owns the stats
/// shape; this function only touches `storage`. Pulled out of
/// `process_line` so the backfill module can replay rotated
/// `Game-*.log` files into the same store without conflating its
/// counters with the live-tail counters.
///
/// `enable_v2_metadata` gates the Phase 4 unknown-line capture pipeline.
/// When false, unrecognised lines are sampled into the legacy `unknowns`
/// table only — exactly the pre-Phase-3 behaviour. When true, they are
/// additionally routed through `classify_or_capture` so a normalised
/// `UnknownLine` record lands in the local SQLite review queue
/// (`unknown_lines`) for the tray's Review pane.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ingest_one_line(
    line: &str,
    storage: &Storage,
    log_source: &str,
    log_source_enum: LogSource,
    line_offset: u64,
    file_sig: Option<&str>,
    remote_rules: &[starstats_core::CompiledRemoteRule],
    enable_v2_metadata: bool,
    own_handle: &str,
    window: Option<&mut InferenceWindow>,
) -> IngestOutcome {
    let Some(parsed) = structural_parse(line) else {
        return IngestOutcome::Skipped;
    };
    // Built-in classifier first; remote rules only run on built-in
    // miss so they can never override or suppress an authoritative
    // classification.
    let event = classify(&parsed).or_else(|| apply_remote_rules(&parsed, remote_rules));
    let Some(event) = event else {
        // Structural parse OK, classifier had no rule. Two paths:
        // 1. event_name is on the noise list → bump noise counter,
        //    don't pollute the actionable unknowns table.
        // 2. event_name is genuinely unknown → record a sample so the
        //    user can see what's missing a rule.
        // No event_name (rare — usually means the line is mid-flight
        // and the structural parser was over-permissive) → skip silently.
        if let Some(event_name) = parsed.event_name {
            // Known-garbage families (VFX/particle chatter) are dropped
            // before the DB noise check: they're never worth a reviewer's
            // time, and the exact engine event names vary too much to
            // enumerate in the noise list. Matches the client-side purge
            // of any such rows captured before this filter existed.
            if starstats_core::is_garbage_line(line) {
                return IngestOutcome::Noise;
            }
            match storage.is_noise(event_name) {
                Ok(true) => return IngestOutcome::Noise,
                Ok(false) => {
                    if let Err(e) =
                        storage.record_unknown(log_source, event_name, line, parsed.body)
                    {
                        tracing::warn!(error = %e, "record_unknown failed");
                    }
                    // Phase 4 capture: only when the v2 pipeline is
                    // enabled. We re-route the unmatched line through
                    // `classify_or_capture` (with no remote rules — the
                    // built-in pass already ran and missed) so the same
                    // normalisation that produces UnknownLine elsewhere
                    // fires here, then upsert into the local cache.
                    if enable_v2_metadata {
                        capture_v2_unknown(storage, &parsed, line, log_source_enum, own_handle);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "is_noise query failed");
                }
            }
        }
        return IngestOutcome::StructuralOnly;
    };
    let Some((event_type, timestamp, payload)) = serialise_event(&event) else {
        return IngestOutcome::Skipped;
    };

    let idempotency_key = idempotency_key(log_source, file_sig, line_offset, line);
    if let Err(e) = storage.insert_event(
        &idempotency_key,
        &event_type,
        &timestamp,
        line,
        &payload,
        log_source,
        line_offset,
    ) {
        tracing::warn!(error = %e, "insert_event failed");
        return IngestOutcome::Skipped;
    }

    // Feed the inference window when v2 is on. Pre-v2 callers (and the
    // v2-flag-off path) pass `None` and skip inference entirely, so
    // a flag-off install behaves identically to before this change.
    if enable_v2_metadata {
        if let Some(window) = window {
            window.push(EventEnvelope {
                idempotency_key: idempotency_key.clone(),
                raw_line: line.to_string(),
                event: Some(event),
                source: log_source_enum,
                source_offset: line_offset,
                metadata: None,
                // Local window only (see the burst-summary push above);
                // sync-path resolution lives in `sync::build_batch`.
                resolved_location: None,
            });
        }
    }

    IngestOutcome::Recognised {
        event_type,
        timestamp,
    }
}

/// Upsert an unmatched line into the local `unknown_lines` review
/// cache. Gated on the `parser.enable_v2_metadata` flag — only the
/// caller in `ingest_one_line` invokes this, and only when the flag
/// is on. Failures are logged at warn and swallowed: the review queue
/// is a best-effort surface, never on the critical ingest path.
///
/// `own_handle` is the player's claimed handle, threaded from the tail
/// loop (empty when the tray is unpaired). It is the one PII input this
/// path can supply, and it must be: `detect_pii` keys own-handle
/// redaction off it, so an empty handle means the Review pane offers no
/// toggle and a submit ships the handle verbatim. An earlier version of
/// this function built the context with `..default()` (empty handle) and
/// claimed in this comment that "PII still gets redacted" — it did not.
///
/// `known_friends`/`game_build` remain absent because the client has no
/// friends list or per-line build to supply; the pipeline degrades
/// honestly (those tokens simply aren't offered) rather than pretending
/// to redact what it cannot identify. `channel` is threaded from the
/// tail loop's `log_source_enum` so the review queue records the real
/// build the line came from rather than flattening to `LogSource::Other`.
fn capture_v2_unknown(
    storage: &Storage,
    parsed: &LogLine<'_>,
    raw_line: &str,
    channel: LogSource,
    own_handle: &str,
) {
    let ctx = CaptureContextOwned {
        channel,
        own_handle: own_handle.to_string(),
        ..CaptureContextOwned::default()
    };
    let outcome = classify_or_capture(parsed, &[], &ctx, raw_line, parsed.timestamp);
    if let ClassifyOutcome::Unknown(unknown) = outcome {
        if let Err(e) = storage.cache_unknown_line(&unknown) {
            tracing::warn!(error = %e, "cache_unknown_line failed");
        }
    }
    // Classified / RemoteMatched outcomes here would indicate a bug —
    // we only reach this helper from the "no built-in, no remote rule"
    // branch above. Drop them silently rather than double-insert.
}

/// Public wrapper around the private channel-derivation logic so the
/// backfill module can compute log_source from a rotated file's path.
pub(crate) fn log_source_from_path(path: &std::path::Path) -> String {
    // `Logs/Game-*.log` lives one level deeper than live `Game.log`,
    // so we look at the **grandparent** directory name when the
    // immediate parent is `Logs`. Otherwise the immediate parent.
    let parent = path.parent();
    let grandparent = parent.and_then(|p| p.parent());
    let segment = match parent.and_then(|p| p.file_name()).and_then(|s| s.to_str()) {
        Some("Logs") => grandparent
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str()),
        Some(name) => Some(name),
        None => None,
    };
    let upper = segment.unwrap_or("OTHER").to_ascii_uppercase();
    match upper.as_str() {
        "LIVE" => "live",
        "PTU" => "ptu",
        "EPTU" => "eptu",
        "HOTFIX" => "hotfix",
        "TECH-PREVIEW" => "tech",
        _ => "other",
    }
    .to_string()
}

/// Head bytes folded into a file signature. Large enough to almost
/// always include a per-session-varying token (a timestamped or
/// build-stamped opening line) so a rotated log yields a different
/// signature, cheap enough to re-read on every drain.
///
/// A file is only signed once it has AT LEAST this many bytes (see
/// [`signature_from_head`]). That guarantees the signed range `[0, N)`
/// is immutable for the rest of the file's life — a growing log never
/// changes its own signature. If the range could grow, the signature
/// (and therefore the F1 idempotency-key salt) would shift drain-to-
/// drain over the first N bytes and re-key already-uploaded lines as
/// duplicates. Below N bytes the file is unsigned (`None`), which the
/// callers treat as "no rotation signal → length heuristic, legacy
/// key" — a tiny, engine-init-only window in a real multi-MB log.
const FILE_SIG_HEAD_BYTES: usize = 512;

/// Stable identity for the physical file currently at a tail path,
/// used to detect launcher rotation — the file being *replaced* — even
/// when the replacement has already grown past our saved byte offset
/// (the case the old `len < offset` shrink check silently missed,
/// causing the reader to seek mid-file and skip a session's opening
/// lines).
///
/// Folds the OS creation time (when the platform reports it) and the
/// first [`FILE_SIG_HEAD_BYTES`] of content into a UUIDv5 — the same
/// deterministic, clock-free hashing convention used for event
/// idempotency keys. Returns `None` until the file has the full
/// [`FILE_SIG_HEAD_BYTES`]-byte head (so the signed range is immutable
/// and the signature can't shift as the log grows); callers treat
/// `None` as "no rotation signal — fall back to length heuristics".
/// Leaves the file cursor repositioned; callers seek explicitly before
/// reading lines.
pub(crate) async fn file_signature(
    file: &mut tokio::fs::File,
    metadata: &std::fs::Metadata,
) -> std::io::Result<Option<String>> {
    file.seek(SeekFrom::Start(0)).await?;
    let mut head = vec![0u8; FILE_SIG_HEAD_BYTES];
    // Loop to a deterministic fill: a single `read` may return a short
    // count, which would make the signature depend on scheduling.
    let mut filled = 0;
    while filled < head.len() {
        let n = file.read(&mut head[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    head.truncate(filled);
    Ok(signature_from_head(created_millis(metadata), &head))
}

/// Blocking twin of [`file_signature`] for the synchronous log
/// re-readers (the reingest command). Produces a byte-identical
/// signature to the async path for the same file, so an event
/// re-ingested from an archive dedupes against the one the live tail
/// already uploaded (F1: both salt the idempotency key with the same
/// physical-file signature).
pub(crate) fn file_signature_sync(
    file: &mut std::fs::File,
    metadata: &std::fs::Metadata,
) -> std::io::Result<Option<String>> {
    use std::io::{Read, Seek, SeekFrom as StdSeekFrom};
    file.seek(StdSeekFrom::Start(0))?;
    let mut head = vec![0u8; FILE_SIG_HEAD_BYTES];
    let mut filled = 0;
    while filled < head.len() {
        let n = file.read(&mut head[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    head.truncate(filled);
    Ok(signature_from_head(created_millis(metadata), &head))
}

/// OS creation time in epoch-millis, when the platform reports it.
/// Folded into the file signature so a replacement file with an
/// identical head still gets a distinct signature.
fn created_millis(metadata: &std::fs::Metadata) -> Option<u128> {
    metadata
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
}

/// Pure signature over the file head + creation time, shared by the
/// async and sync readers so both produce the same key for the same
/// bytes. Returns `None` until the file has the full
/// [`FILE_SIG_HEAD_BYTES`]-byte head, so the signed range is immutable
/// and the signature never shifts as the log grows (see the const's
/// docs). `head` is whatever the caller managed to read (`<= N`).
fn signature_from_head(created_millis: Option<u128>, head: &[u8]) -> Option<String> {
    if head.len() < FILE_SIG_HEAD_BYTES {
        return None;
    }
    use uuid::Uuid;
    let mut payload = format!("{}:", created_millis.unwrap_or(0)).into_bytes();
    payload.extend_from_slice(head);
    Some(Uuid::new_v5(&Uuid::NAMESPACE_OID, &payload).to_string())
}

/// Decide the byte offset to resume a tail from, given the stored
/// cursor and the current file's signature + length.
///
/// * Signature known on both sides and changed → the file was replaced
///   (launcher rotation); restart at the head.
/// * Otherwise, if the file is shorter than the saved offset it was
///   truncated in place → restart at the head.
/// * Otherwise resume at the saved offset.
///
/// A `None` on either signature means "no rotation signal available"
/// (legacy cursor row, or a file too short to sign yet) and the result
/// falls back to the length-only heuristic — never a false reset.
pub(crate) fn resolve_resume_offset(
    stored_offset: u64,
    stored_sig: Option<&str>,
    current_sig: Option<&str>,
    current_len: u64,
) -> u64 {
    if let (Some(a), Some(b)) = (stored_sig, current_sig) {
        if a != b {
            return 0;
        }
    }
    if current_len < stored_offset {
        return 0;
    }
    stored_offset
}

/// Stable per-line key. Same byte offset + same content in the same
/// physical file always produces the same key, so a re-tail of that
/// file (e.g. after a crash recovery) hits the UNIQUE constraint
/// instead of double-inserting. UUIDv5 over the SHA-1 of
/// (source || [file_sig ||] offset || line) — 36 chars, deterministic,
/// no clock dependency.
///
/// `file_sig` (F1): the byte offset resets to 0 on every log rotation,
/// so `(source, offset, line)` alone collides across sessions — a
/// static banner line at offset N in session 1 and the same line at
/// offset N in session 2 hash identically, and the second is silently
/// dropped as a false duplicate. Salting with the physical file's
/// signature (see [`file_signature`]) makes the two sessions'
/// occurrences distinct.
///
/// When `file_sig` is `None` the salt is omitted and the key is
/// **byte-identical to the pre-F1 format**. This is deliberate: the
/// backfill, reingest, and org-connector paths don't carry a
/// signature, and preserving their exact keys keeps them idempotent
/// with anything already uploaded — the scheme change never manufactures
/// a duplicate on the server for a line it has already stored.
pub(crate) fn idempotency_key(
    log_source: &str,
    file_sig: Option<&str>,
    offset: u64,
    line: &str,
) -> String {
    use uuid::Uuid;
    let payload = match file_sig {
        Some(sig) => format!("{log_source}:{sig}:{offset}:{line}"),
        None => format!("{log_source}:{offset}:{line}"),
    };
    Uuid::new_v5(&Uuid::NAMESPACE_OID, payload.as_bytes()).to_string()
}

/// Stable key for a synthesised inferred event. Deterministic over
/// `(rule_id, trigger_idempotency_key)` so the same trigger replayed
/// through the inference window (e.g. on a re-drain) collapses to one
/// row. Mirrors the UUIDv5 convention used for observed lines so a
/// downstream consumer that hashes envelope ids doesn't see two
/// different key shapes on the wire.
pub(crate) fn inferred_idempotency_key(rule_id: &str, trigger_key: &str) -> String {
    use uuid::Uuid;
    let ns = Uuid::NAMESPACE_OID;
    let payload = format!("inferred:{rule_id}:{trigger_key}");
    Uuid::new_v5(&ns, payload.as_bytes()).to_string()
}

/// Map the string `log_source` (used by storage and the legacy
/// pre-v2 wire shape) back to the typed `LogSource` enum the wire
/// format expects. Inverse of `parse_source` in `sync.rs`. Lives here
/// rather than in `sync.rs` so both modules don't need to import each
/// other; collapsing the two helpers is tracked separately.
pub(crate) fn log_source_enum_from_str(s: &str) -> LogSource {
    match s {
        "live" => LogSource::Live,
        "ptu" => LogSource::Ptu,
        "eptu" => LogSource::Eptu,
        "hotfix" => LogSource::Hotfix,
        "tech" => LogSource::Tech,
        _ => LogSource::Other,
    }
}

fn serialise_event(event: &GameEvent) -> Option<(String, String, String)> {
    let payload = serde_json::to_string(event).ok()?;
    let value: serde_json::Value = serde_json::from_str(&payload).ok()?;
    let event_type = value.get("type")?.as_str()?.to_string();
    let timestamp = value
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some((event_type, timestamp, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact shape the game writes: the notification's quoted text is
    /// split across two physical lines, so MissionId/ObjectiveId land on the
    /// continuation and the first line has an UNTERMINATED quote.
    #[test]
    fn stitches_split_hud_notification_and_keeps_first_offset() {
        let lines = vec![
            (
                "<2026-07-26T13:59:21.014Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Objective Complete: Go to a debris field above Euterpe".to_string(),
                1000u64,
            ),
            (
                "<2026-07-26T13:59:21.014Z> : \" [15] to queue. New queue size: 1, MissionId: [7de35808-d909-4a6d-affe-edadf3e6fe77], ObjectiveId: [2432e890-93a3-0c46-a8a5-c7bb4915881f] [Team_CoreGameplayFeatures]".to_string(),
                1140u64,
            ),
        ];
        let out = stitch_multiline_records(lines);
        assert_eq!(out.len(), 1, "the two halves must become ONE record");
        // The record must carry the FIRST line's offset — a resumed tail
        // seeks to this value, so using the continuation's offset would
        // re-emit or skip records across a restart.
        assert_eq!(out[0].1, 1000);
        assert!(out[0].0.contains("MissionId: [7de35808"));
        assert!(out[0].0.contains("ObjectiveId: [2432e890"));
        // The join inserts a single space between the halves rather than
        // fusing their tokens together (a mashed-together "above Euterpe:
        // \" [15]" with no separator was the actual corrupted shape stored
        // before this fix) — and rather than a literal `\n`, which looks
        // byte-faithful but breaks `SHELL_RE`'s `.*$` and makes the merged
        // record fail to parse at all (see the function's doc comment).
        assert!(out[0].0.contains("above Euterpe : \" [15]"));
        // Exact-match the whole merged record so a future change to the
        // join separator or the continuation-body stripping is caught even
        // if it doesn't happen to break one of the `contains` checks above.
        assert_eq!(
            out[0].0,
            "<2026-07-26T13:59:21.014Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Objective Complete: Go to a debris field above Euterpe : \" [15] to queue. New queue size: 1, MissionId: [7de35808-d909-4a6d-affe-edadf3e6fe77], ObjectiveId: [2432e890-93a3-0c46-a8a5-c7bb4915881f] [Team_CoreGameplayFeatures]"
        );
    }

    /// A general line-joiner swallowed ~106,000 unrelated lines out of
    /// 128,031 in a real-log trial. Only unterminated HUD-notification
    /// lines may be joined; everything else passes through untouched.
    #[test]
    fn leaves_unrelated_lines_untouched() {
        let lines = vec![
            ("<2026-07-26T13:50:09.490Z> [Notice] <ContextEstablisherTaskFinished> establisher=\"CReplicationModel\" state=eCVS_ChangeServer(3)".to_string(), 10u64),
            ("<2026-07-26T13:50:57.171Z> [+] [CIG] {Join PU} [0] id[5caa] status[1] port[64307]".to_string(), 20u64),
            ("some raw engine line with no timestamp at all".to_string(), 30u64),
            ("<2026-07-26T13:57:59.955Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Contract Accepted:  Combat Gauntlet - Scenario #5: \" [9] to queue. MissionId: [7de35808-d909-4a6d-affe-edadf3e6fe77], ObjectiveId: []".to_string(), 40u64),
        ];
        let out = stitch_multiline_records(lines.clone());
        assert_eq!(out.len(), lines.len(), "nothing may be merged here");
        for (i, (s, off)) in out.iter().enumerate() {
            assert_eq!(*s, lines[i].0);
            assert_eq!(*off, lines[i].1);
        }
    }

    /// A truncated tail (continuation not yet written, or a malformed line)
    /// must not consume the rest of the buffer.
    #[test]
    fn unterminated_record_with_no_continuation_passes_through() {
        let lines = vec![
            ("<2026-07-26T13:59:21.014Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Objective Complete: dangling".to_string(), 5u64),
        ];
        let out = stitch_multiline_records(lines);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, 5);
        // Unchanged — it will simply fail to classify, exactly as today.
        assert!(out[0].0.ends_with("dangling"));
    }

    /// Real logs contain records spanning THREE physical lines (9 of 238
    /// joins in the 40-log corpus). The join must keep going until the
    /// quote closes, within the cap.
    #[test]
    fn stitches_three_line_record() {
        let lines = vec![
            ("<2026-07-26T00:00:00.000Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Part one".to_string(), 0u64),
            ("<2026-07-26T00:00:00.000Z> part two".to_string(), 50u64),
            ("<2026-07-26T00:00:00.000Z> : \" [3] to queue. MissionId: [aaaaaaaa-0000-0000-0000-000000000000]".to_string(), 90u64),
        ];
        let out = stitch_multiline_records(lines);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, 0);
        assert!(out[0].0.contains("MissionId: [aaaaaaaa"));
        // Exact-match: two join points, so two inserted space separators.
        assert_eq!(
            out[0].0,
            "<2026-07-26T00:00:00.000Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Part one part two : \" [3] to queue. MissionId: [aaaaaaaa-0000-0000-0000-000000000000]"
        );
    }

    /// FIX: quote-parity alone can't distinguish "unterminated" from "a
    /// complete record that happens to contain an odd number of quotes"
    /// (an apostrophe-adjacent name, a stray literal quote in the banner
    /// text). If the line immediately after a candidate is itself a
    /// well-formed record start (`<ts> [Level] ...`), the candidate must
    /// not absorb it.
    #[test]
    fn candidate_does_not_swallow_a_following_well_formed_record() {
        let lines = vec![
            (
                // Three quotes (odd) despite being a complete, if oddly
                // quoted, notification.
                "<2026-07-27T00:00:00.000Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Text with a rogue \" quote inside: \" [1] to queue. MissionId: []".to_string(),
                0u64,
            ),
            (
                "<2026-07-27T00:00:01.000Z> [Notice] <SomeOtherEvent> a real, unrelated record".to_string(),
                80u64,
            ),
        ];
        let out = stitch_multiline_records(lines.clone());
        assert_eq!(
            out.len(),
            2,
            "a mis-detected candidate must not swallow a well-formed following record"
        );
        assert_eq!(out[0], lines[0]);
        assert_eq!(out[1], lines[1]);
    }

    /// Conservation property: no input line's content may be silently
    /// dropped. Walks input and output in lockstep — every output record
    /// must equal the space-joined reconstruction of a contiguous run of
    /// consecutive, not-yet-consumed input lines (the first line's untouched
    /// full text, then each following line's continuation body, i.e. itself
    /// minus its own redundant leading `<ts> `). A future predicate change
    /// that starts dropping lines instead of just reshaping them would
    /// break this even with every other test green.
    ///
    /// Reconstructs the expected merged string directly (rather than
    /// splitting the actual output on the join separator) because the
    /// separator is a plain space — see `stitch_multiline_records`'s doc
    /// comment for why it can't be `\n` — and a space is not a reliable
    /// split point: it also occurs inside ordinary line content.
    #[test]
    fn stitching_conserves_all_input_content_and_offsets_increase() {
        fn expected_continuation_body(s: &str) -> &str {
            s.strip_prefix('<')
                .and_then(|rest| rest.split_once("> "))
                .map(|(_, body)| body)
                .unwrap_or(s)
        }

        /// The expected merged text for a record starting at `lines[start]`
        /// and consuming `consumed` input lines: the first line untouched,
        /// then each following line's continuation body, joined by the
        /// same single-space separator `stitch_multiline_records` uses.
        fn expected_joined(lines: &[(String, u64)], start: usize, consumed: usize) -> String {
            let mut merged = lines[start].0.clone();
            for k in 1..consumed {
                merged.push(' ');
                merged.push_str(expected_continuation_body(&lines[start + k].0));
            }
            merged
        }

        let lines = vec![
            // Plain pass-through.
            (
                "<2026-07-27T00:00:00.000Z> [Notice] <SomeEvent> unrelated body".to_string(),
                0u64,
            ),
            // Two-line record.
            (
                "<2026-07-27T00:00:01.000Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Part A".to_string(),
                60u64,
            ),
            (
                "<2026-07-27T00:00:01.000Z> : \" [1] to queue. MissionId: [aaaaaaaa-0000-0000-0000-000000000000]".to_string(),
                130u64,
            ),
            // Three-line record.
            (
                "<2026-07-27T00:00:02.000Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Part B".to_string(),
                220u64,
            ),
            (
                "<2026-07-27T00:00:02.000Z> continues here".to_string(),
                280u64,
            ),
            (
                "<2026-07-27T00:00:02.000Z> : \" [2] to queue. MissionId: [bbbbbbbb-0000-0000-0000-000000000000]".to_string(),
                340u64,
            ),
            // Never-closing candidate immediately followed by a
            // well-formed record — must pass through untouched (both the
            // "no continuation ever arrives" case and the FIX 5 guard).
            (
                "<2026-07-27T00:00:03.000Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Dangling forever".to_string(),
                420u64,
            ),
            (
                "<2026-07-27T00:00:04.000Z> [Notice] <SomeEvent> trailing body".to_string(),
                500u64,
            ),
        ];

        let out = stitch_multiline_records(lines.clone());

        for w in out.windows(2) {
            assert!(
                w[0].1 < w[1].1,
                "output offsets must strictly increase: {out:?}"
            );
        }

        let mut next_input = 0usize;
        for (record, _offset) in &out {
            assert!(
                next_input < lines.len(),
                "output has more records than the input could produce"
            );
            // Find the smallest `consumed` whose expected reconstruction
            // matches this record exactly. A shorter reconstruction is
            // always a strict prefix-plus-more-to-come of a longer one (the
            // continuation bodies are non-empty in every fixture line
            // above), so the first match is unambiguously the real count —
            // never a false positive from a shorter run happening to equal
            // the full merged text.
            let remaining = lines.len() - next_input;
            let consumed = (1..=remaining)
                .find(|&c| expected_joined(&lines, next_input, c) == *record)
                .unwrap_or_else(|| {
                    panic!(
                        "record does not match any contiguous run of input lines \
                         starting at index {next_input}: {record:?}"
                    )
                });
            next_input += consumed;
        }
        assert_eq!(
            next_input,
            lines.len(),
            "every input line must be accounted for exactly once — none dropped, none duplicated"
        );
    }

    /// Quote-parity alone is NOT a safe candidate test — it must be gated by
    /// the `Added notification "` substring. Measured across 301,115 real
    /// log lines: odd-quote lines containing that substring: 238 (the real
    /// candidates); odd-quote lines that do NOT contain it: 3,534 — a 15x
    /// over-trigger if the substring check were dropped. The worst offenders
    /// are exactly the shapes below: a `<UpdateNotificationItem>` chat line
    /// (one bare quote from an apostrophe-adjacent player/channel name) and
    /// a stitcher continuation line (itself carrying an odd quote count).
    /// Both must pass through untouched, and the first must NOT absorb the
    /// second.
    #[test]
    fn odd_quote_non_hud_line_is_not_a_stitch_candidate() {
        let lines = vec![
            (
                "<2026-05-20T18:15:13.656Z> [Notice] <UpdateNotificationItem> Notification \"You have joined channel 'Aegis Reclaimer : TheCodeSaiyan'.".to_string(),
                200u64,
            ),
            (
                "<2026-05-20T18:15:13.652Z> : \" [12] to queue. New queue size: 1, MissionId: [00000000-0000-0000-0000-000000000000], ObjectiveId: [".to_string(),
                340u64,
            ),
        ];
        let out = stitch_multiline_records(lines.clone());
        assert_eq!(
            out.len(),
            2,
            "an odd-quote non-HUD line must not trigger a join"
        );
        assert_eq!(out[0], lines[0]);
        assert_eq!(out[1], lines[1]);
    }

    /// Regression guard for a Critical shipped alongside the stitcher: every
    /// test above calls `stitch_multiline_records` directly and asserts on
    /// strings, so none of them exercise `structural_parse`, `classify`, or
    /// storage. That gap let this ship — joining the two halves with `\n`
    /// (to stop token-mashing) made `SHELL_RE`'s `(?P<rest>.*)$` fail to
    /// match the merged record at all (`regex`'s `.` doesn't span `\n`
    /// unless `dot_matches_new_line` is set, which this crate never sets),
    /// so `structural_parse` returned `None` and the stitched record was
    /// silently `IngestOutcome::Skipped` — recovering ZERO records while
    /// every existing stitcher test stayed green.
    ///
    /// Feeds the exact split two-line HUD notification pair (same shape as
    /// `stitches_split_hud_notification_and_keeps_first_offset`) through
    /// `process_buffer` — not `stitch_multiline_records` directly — so the
    /// stitched output must actually survive structural parsing and
    /// classification and land in storage as a `hud_notification` row
    /// carrying the mission/objective ids that only exist on the
    /// continuation line.
    #[test]
    fn stitched_hud_notification_reaches_storage_via_process_buffer() {
        let (_dir, storage) = open_temp_storage();
        let rules = RuleCache::new();
        let mut window = InferenceWindow::default();
        let buffer = vec![
            (
                "<2026-07-26T13:59:21.014Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Objective Complete: Go to a debris field above Euterpe".to_string(),
                1000u64,
            ),
            (
                "<2026-07-26T13:59:21.014Z> : \" [15] to queue. New queue size: 1, MissionId: [7de35808-d909-4a6d-affe-edadf3e6fe77], ObjectiveId: [2432e890-93a3-0c46-a8a5-c7bb4915881f] [Team_CoreGameplayFeatures]".to_string(),
                1140u64,
            ),
        ];
        run_two_line_drain(
            &storage,
            &rules,
            "live",
            LogSource::Live,
            false,
            &mut window,
            &buffer,
        );

        let rows = storage.recent_events(10).expect("recent_events");
        let hud_row = rows
            .iter()
            .find(|r| r.event_type == "hud_notification")
            .unwrap_or_else(|| {
                panic!(
                    "expected a hud_notification row recovered from the stitched record; \
                     got event types: {:?}",
                    rows.iter().map(|r| &r.event_type).collect::<Vec<_>>()
                )
            });

        let payload: serde_json::Value =
            serde_json::from_str(&hud_row.payload_json).expect("payload must be valid JSON");
        assert_eq!(
            payload.get("mission_id").and_then(|v| v.as_str()),
            Some("7de35808-d909-4a6d-affe-edadf3e6fe77"),
            "mission_id (only present on the continuation line) must survive the stitch+parse; got {payload}",
        );
        assert_eq!(
            payload.get("objective_id").and_then(|v| v.as_str()),
            Some("2432e890-93a3-0c46-a8a5-c7bb4915881f"),
            "objective_id (only present on the continuation line) must survive the stitch+parse; got {payload}",
        );
    }

    /// Opt-in: runs only when STARSTATS_LOG_CORPUS points at a directory of
    /// real Game.log files (e.g. StarCitizen/LIVE/logbackups). Synthetic
    /// fixtures cannot prove the stitcher behaves on real data, and this
    /// repo has been bitten by exactly that gap before.
    ///
    /// Asserts the two properties that matter:
    ///   1. Stitching is SURGICAL — it must absorb only a tiny fraction of
    ///      lines. A general joiner collapsed 128,031 to 22,118 in trial.
    ///   2. It RECOVERS Objective Complete records that carry a MissionId,
    ///      which is zero without stitching.
    ///
    /// `recovered` counts records that actually run the full pipeline —
    /// `structural_parse` then `classify` — and land as a `HudNotification`
    /// with a non-`None` `mission_id`. This used to be a `str::contains`
    /// check over the stitcher's raw output strings, which is
    /// mathematically incapable of detecting a classification break: it
    /// reported hundreds of "recovered" records for a build where the `\n`
    /// join separator made every one of them fail `structural_parse` and
    /// get silently dropped as `IngestOutcome::Skipped`. Only a check that
    /// exercises the same parse path production code takes can prove
    /// anything was actually recovered.
    #[test]
    #[ignore = "requires STARSTATS_LOG_CORPUS"]
    fn stitcher_on_real_log_corpus() {
        let Ok(dir) = std::env::var("STARSTATS_LOG_CORPUS") else {
            eprintln!("STARSTATS_LOG_CORPUS unset — skipping");
            return;
        };
        let mut total_in = 0usize;
        let mut total_out = 0usize;
        let mut recovered = 0usize;
        for entry in std::fs::read_dir(&dir).expect("read corpus dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }
            let body = std::fs::read_to_string(&path).unwrap_or_else(|_| {
                String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned()
            });
            let lines: Vec<(String, u64)> = body
                .lines()
                .enumerate()
                .map(|(i, l)| (l.to_string(), i as u64))
                .collect();
            total_in += lines.len();
            let out = stitch_multiline_records(lines);
            total_out += out.len();
            for (rec, _) in &out {
                let Some(parsed) = structural_parse(rec) else {
                    continue;
                };
                let Some(GameEvent::HudNotification(h)) = classify(&parsed) else {
                    continue;
                };
                if h.text.starts_with("Objective Complete") && h.mission_id.is_some() {
                    recovered += 1;
                }
            }
        }
        assert!(total_in > 0, "corpus produced no lines — wrong directory?");
        let absorbed = total_in - total_out;
        // Surgical: well under 1% of lines. Measured on a 280-log corpus:
        // 3,702,127 lines -> 2,943 absorbed = 0.0795%. Bound at 0.2% (5x
        // the measured rate) so a regression absorbing meaningfully more
        // still fails loudly; the naive joiner absorbed ~83%.
        assert!(
            absorbed * 500 < total_in,
            "stitcher absorbed {absorbed} of {total_in} lines — far too aggressive"
        );
        // Recovery: without stitching this is zero.
        assert!(
            recovered > 0,
            "no mission-linked Objective Complete records recovered"
        );
        eprintln!("corpus: {total_in} lines -> {total_out} records ({absorbed} absorbed), {recovered} Objective Complete recovered");
    }

    #[test]
    fn carry_boundary_holds_tail_and_never_splits_a_burst() {
        // No bursts: carry exactly the trailing max_carry lines.
        assert_eq!(carry_boundary(1000, &[], 256), 744);
        // Buffer at or below the carry window: carry everything, no commit.
        assert_eq!(carry_boundary(100, &[], 256), 0);
        assert_eq!(carry_boundary(256, &[], 256), 0);
        // A burst fully before the tail is committed (cut unchanged).
        assert_eq!(carry_boundary(1000, &[(100, 150)], 256), 744);
        // A burst reaching into the tail is carried WHOLE (cut -> anchor).
        assert_eq!(carry_boundary(1000, &[(700, 800)], 256), 700);
        // A burst whose end lands exactly on the tail start still carries whole.
        assert_eq!(carry_boundary(1000, &[(700, 744)], 256), 700);
        // A burst entirely inside the tail: cut stays at the tail start
        // (the burst is in the carried region anyway).
        assert_eq!(carry_boundary(1000, &[(760, 790)], 256), 744);
        // Multiple bursts: only the tail-touching one pulls the cut back;
        // the earlier complete burst is committed.
        assert_eq!(carry_boundary(1000, &[(100, 150), (700, 900)], 256), 700);
    }

    #[test]
    fn burst_buffer_ranges_maps_a_real_burst_to_buffer_indices() {
        // A non-burst prefix line, then a 3-line loadout burst — exercises
        // the parse -> detect_bursts -> buffer-index remap that feeds
        // carry_boundary (so a straddling burst is carried by anchor).
        const A1: &str = concat!(
            "<2026-05-03T17:52:57.219Z> [Notice] <AttachmentReceived> Player[X] ",
            "Attachment[a_undersuit_01, a_undersuit_01, 1] Status[persistent] ",
            "Port[Armor_Undersuit] Elapsed[1.0] [Team_CoreGameplayFeatures][Inventory]"
        );
        const A2: &str = concat!(
            "<2026-05-03T17:52:57.220Z> [Notice] <AttachmentReceived> Player[X] ",
            "Attachment[a_helmet_01, a_helmet_01, 2] Status[persistent] ",
            "Port[Armor_Helmet] Elapsed[1.1] [Team_CoreGameplayFeatures][Inventory]"
        );
        const A3: &str = concat!(
            "<2026-05-03T17:52:57.221Z> [Notice] <AttachmentReceived> Player[X] ",
            "Attachment[a_pistol_01, a_pistol_01, 3] Status[persistent] ",
            "Port[WeaponRight] Elapsed[1.2] [Team_CoreGameplayFeatures][Inventory]"
        );
        let rules = crate::burst_rules::builtin_burst_rules();
        let buffer = vec![
            (
                "<2026-05-03T17:52:00.000Z> [Notice] <SomeOther> body".to_string(),
                0u64,
            ),
            (A1.to_string(), 1u64),
            (A2.to_string(), 2u64),
            (A3.to_string(), 3u64),
        ];
        let ranges = burst_buffer_ranges(&buffer, &rules);
        assert_eq!(
            ranges,
            vec![(1, 3)],
            "the 3-line burst maps to buffer indices 1..=3 (prefix line excluded)"
        );
    }
    use tempfile::TempDir;

    /// Line shaped like a normal `<Notice>` event with an event_name
    /// that won't match any built-in classifier or noise-list entry —
    /// so `classify` returns None and the unmatched branch runs.
    const MYSTERY_LINE: &str = "<2026-05-17T14:02:30.000Z> [Notice] <SomeUnknownEventNameForFlagTest> body fields here [Team_Unknown]";

    fn open_temp_storage() -> (TempDir, Storage) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("test.db");
        let storage = Storage::open(&path).expect("open storage");
        (dir, storage)
    }

    // --- F2: rotation-aware resume-offset resolution -------------------
    // A replacement `Game.log` (launcher rotation) that has already
    // grown PAST the saved offset must be re-read from the head, not
    // seeked into mid-file. The old shrink-only check (`len < offset`)
    // missed this and silently skipped a session's opening lines.

    #[test]
    fn resume_same_file_that_grew_keeps_offset() {
        // Same signature, file grew → resume where we left off.
        assert_eq!(
            resolve_resume_offset(100, Some("sig-a"), Some("sig-a"), 500),
            100
        );
    }

    #[test]
    fn resume_rotated_file_that_outgrew_offset_resets_to_head() {
        // THE F2 BUG: new file, longer than the old offset. Signature
        // changed → it's a different file → must restart at 0.
        assert_eq!(
            resolve_resume_offset(100, Some("sig-a"), Some("sig-b"), 500),
            0
        );
    }

    #[test]
    fn resume_same_length_replacement_resets_to_head() {
        // Replacement file that happens to be exactly the old length.
        // `len == offset` used to short-circuit as "nothing new"; the
        // signature change catches it.
        assert_eq!(
            resolve_resume_offset(100, Some("sig-a"), Some("sig-b"), 100),
            0
        );
    }

    #[test]
    fn resume_truncated_in_place_resets_to_head() {
        // Same signature, file shorter than offset → truncated → 0.
        assert_eq!(
            resolve_resume_offset(500, Some("sig-a"), Some("sig-a"), 100),
            0
        );
    }

    #[test]
    fn resume_legacy_row_without_signature_falls_back_to_length() {
        // Legacy cursor (pre-migration) has no stored signature: we
        // can't prove rotation, so fall back to the old len-based
        // heuristic — grew keeps the offset, shrank resets.
        assert_eq!(resolve_resume_offset(100, None, Some("sig-a"), 500), 100);
        assert_eq!(resolve_resume_offset(500, None, Some("sig-a"), 100), 0);
    }

    #[test]
    fn resume_unsignable_current_file_falls_back_to_length() {
        // Current file too short/empty to sign (current_sig None):
        // no rotation signal available → len-based fallback only.
        assert_eq!(resolve_resume_offset(100, Some("sig-a"), None, 500), 100);
        assert_eq!(resolve_resume_offset(500, Some("sig-a"), None, 100), 0);
    }

    // --- F1: file-signature-salted idempotency keys -------------------

    #[test]
    fn idempotency_key_without_signature_matches_legacy_format() {
        // The `None` branch MUST be byte-identical to the pre-F1 key so
        // events already on the server (and the backfill/reingest/org
        // paths that never carry a signature) keep the same key — no
        // duplicate rows across the upgrade.
        let got = idempotency_key("live", None, 100, "L");
        let legacy = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b"live:100:L").to_string();
        assert_eq!(got, legacy);
    }

    #[test]
    fn idempotency_key_salt_distinguishes_files_at_same_offset() {
        // F1 fix: the same line at the same offset in two different
        // sessions (different file signatures) must NOT collide.
        let a = idempotency_key("live", Some("sigA"), 100, "L");
        let b = idempotency_key("live", Some("sigB"), 100, "L");
        assert_ne!(
            a, b,
            "identical line+offset in different files must not collide"
        );
        assert_ne!(
            a,
            idempotency_key("live", None, 100, "L"),
            "salted key must differ from the legacy key"
        );
        // Stable within one file: a crash-recovery re-tail dedupes.
        assert_eq!(a, idempotency_key("live", Some("sigA"), 100, "L"));
    }

    #[tokio::test]
    async fn sync_and_async_file_signatures_agree() {
        // Load-bearing invariant: the live tail (async) and the reingest
        // command (sync) must derive the SAME signature for the same
        // bytes, or re-ingested events salt to a different key and
        // duplicate the live-tailed rows instead of deduping.
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("Game.log");
        // Must exceed FILE_SIG_HEAD_BYTES (512) or the file is unsigned.
        let content: String = (0..40)
            .map(|i| format!("<2026-05-17T14:02:30.000Z> [Notice] <Init> header line {i}\n"))
            .collect();
        assert!(
            content.len() > FILE_SIG_HEAD_BYTES,
            "test file must be signable"
        );
        std::fs::write(&path, content.as_bytes()).expect("write");

        let mut afile = tokio::fs::File::open(&path).await.expect("async open");
        let ameta = afile.metadata().await.expect("async meta");
        let async_sig = file_signature(&mut afile, &ameta).await.expect("async sig");

        let mut sfile = std::fs::File::open(&path).expect("sync open");
        let smeta = sfile.metadata().expect("sync meta");
        let sync_sig = file_signature_sync(&mut sfile, &smeta).expect("sync sig");

        assert!(async_sig.is_some(), "a non-empty file must sign");
        assert_eq!(
            async_sig, sync_sig,
            "sync and async signatures must match or cross-path dedup breaks"
        );
    }

    #[tokio::test]
    async fn file_signature_is_stable_as_file_grows_past_head() {
        // Regression: the signature MUST NOT change as an append-only
        // log grows past its head window. If it did, the F1 key salt
        // would shift drain-to-drain and re-key already-uploaded lines
        // as duplicates. Guaranteed by signing only the immutable
        // `[0, FILE_SIG_HEAD_BYTES)` prefix.
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("Game.log");
        let head: String = (0..40)
            .map(|i| format!("stable head filler line number {i}\n"))
            .collect();
        assert!(
            head.len() >= FILE_SIG_HEAD_BYTES,
            "head must fill the window"
        );
        std::fs::write(&path, head.as_bytes()).expect("write head");

        let mut f1 = tokio::fs::File::open(&path).await.expect("open 1");
        let m1 = f1.metadata().await.expect("meta 1");
        let sig_small = file_signature(&mut f1, &m1).await.expect("sig 1");

        // Grow the file by APPENDING (as the game engine does) rather
        // than rewriting it — same inode, same creation time, same head
        // bytes; only the tail after the window changes.
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("open append");
            for i in 0..500 {
                writeln!(f, "appended line {i}").expect("append");
            }
        }

        let mut f2 = tokio::fs::File::open(&path).await.expect("open 2");
        let m2 = f2.metadata().await.expect("meta 2");
        let sig_grown = file_signature(&mut f2, &m2).await.expect("sig 2");

        assert_eq!(
            sig_small, sig_grown,
            "signature must be stable as the log grows past its head window"
        );
    }

    #[test]
    fn ingest_one_line_v2_flag_off_skips_unknown_line_cache() {
        // Flag off: unmatched lines land in the legacy `unknowns`
        // sample table but never reach the Phase 4 `unknown_lines`
        // review cache. This is the pre-Phase-3 behaviour we promise
        // a flag-off install gets.
        let (_dir, storage) = open_temp_storage();
        let outcome = ingest_one_line(
            MYSTERY_LINE,
            &storage,
            "live",
            LogSource::Live,
            0,
            None,
            &[],
            false,
            "",
            None,
        );
        assert!(matches!(outcome, IngestOutcome::StructuralOnly));
        assert_eq!(
            storage.count_unknown_lines(0).expect("count"),
            0,
            "v2 capture must not fire when the flag is off",
        );
    }

    #[test]
    fn ingest_one_line_v2_flag_on_captures_to_unknown_line_cache() {
        // Flag on: the same line additionally lands in the v2 review
        // cache so the tray's Review pane can surface it.
        let (_dir, storage) = open_temp_storage();
        let outcome = ingest_one_line(
            MYSTERY_LINE,
            &storage,
            "live",
            LogSource::Live,
            0,
            None,
            &[],
            true,
            "",
            None,
        );
        assert!(matches!(outcome, IngestOutcome::StructuralOnly));
        assert_eq!(
            storage.count_unknown_lines(0).expect("count"),
            1,
            "v2 capture must persist exactly one unknown-line row",
        );
    }

    #[test]
    fn ingest_one_line_drops_garbage_before_capture() {
        // A VFX/particle line is structurally valid and unrecognised, so
        // without the garbage gate it would land in the review queue. It
        // must instead be counted as noise and never captured.
        let (_dir, storage) = open_temp_storage();
        let vfx =
            "<2026-05-02T21:15:03.053Z> [Error] <SomeVfxThing> spawning effect [Team_VFX][VFX]";
        let outcome = ingest_one_line(
            vfx,
            &storage,
            "live",
            LogSource::Live,
            0,
            None,
            &[],
            true,
            "",
            None,
        );
        assert!(
            matches!(outcome, IngestOutcome::Noise),
            "garbage must be classed as noise",
        );
        assert_eq!(
            storage.count_unknown_lines(0).expect("count"),
            0,
            "garbage must never reach the review queue",
        );
    }

    #[test]
    fn ingest_one_line_v2_flag_on_recognised_line_does_not_double_capture() {
        // Built-in classifier matches → the unmatched branch never
        // runs, so the flag is a no-op for recognised lines. Asserts
        // the gate doesn't leak into the happy path.
        let (_dir, storage) = open_temp_storage();
        // Use the same PlayerDeath fixture as core's classifier tests
        // — a line that's guaranteed to classify cleanly.
        let known_line = "<2026-05-01T18:46:15.085Z> [Notice] <Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]> Item 'body_01_noMagicPocket_9754924365641 - Class(body_01_noMagicPocket) - Context(Streamable Runtime-spawned) - Socpak()', Recorded data is: Port Name 'Body_ItemPort', Class GUID: 'dbaa8a7d-755f-4104-8b24-7b58fd1e76f6', KeptId: '9754924365641' [Team_CoreGameplayFeatures][Unknown]";
        let outcome = ingest_one_line(
            known_line,
            &storage,
            "live",
            LogSource::Live,
            0,
            None,
            &[],
            true,
            "",
            None,
        );
        assert!(matches!(outcome, IngestOutcome::Recognised { .. }));
        assert_eq!(
            storage.count_unknown_lines(0).expect("count"),
            0,
            "classified lines must never land in the unknown-line cache",
        );
    }

    #[test]
    fn capture_v2_unknown_redacts_own_handle() {
        // The parser-submission path must surface the user's own handle
        // as a redactable PII token. Before this fix, capture_v2_unknown
        // built its CaptureContextOwned with an empty own_handle, so
        // detect_pii emitted zero OwnHandle tokens: the Review pane
        // offered no redaction toggle and a submit shipped the handle
        // verbatim, while a doc comment claimed "PII still gets redacted".
        // A captured line naming the player must yield an OwnHandle token
        // defaulting to redacted.
        use starstats_core::unknown_lines::PiiKind;
        let (_dir, storage) = open_temp_storage();
        let line = "<2026-05-17T14:02:30.000Z> [Notice] <SomeUnknownEventNameForPiiTest> actor TheCodeSaiyan did a thing [Team_Unknown]";
        let outcome = ingest_one_line(
            line,
            &storage,
            "live",
            LogSource::Live,
            0,
            None,
            &[],
            true,
            "TheCodeSaiyan",
            None,
        );
        assert!(matches!(outcome, IngestOutcome::StructuralOnly));
        let rows = storage.list_unknown_lines(0).expect("list");
        assert_eq!(rows.len(), 1, "expected one captured row");
        let own = rows[0]
            .detected_pii
            .iter()
            .find(|t| t.kind == PiiKind::OwnHandle)
            .expect("own-handle PII token must be present so the Review pane can offer redaction");
        assert!(
            own.default_redact,
            "the user's own handle must default to redacted",
        );
    }

    #[test]
    fn capture_v2_unknown_records_real_channel() {
        // Regression for the hardcoded `LogSource::Other` bug: an
        // unrecognised line ingested via a PTU tail must land in the
        // review cache with `channel = LogSource::Ptu`, not `Other`.
        let (_dir, storage) = open_temp_storage();
        let outcome = ingest_one_line(
            MYSTERY_LINE,
            &storage,
            "ptu",
            LogSource::Ptu,
            0,
            None,
            &[],
            true,
            "",
            None,
        );
        assert!(matches!(outcome, IngestOutcome::StructuralOnly));
        let rows = storage.list_unknown_lines(0).expect("list");
        assert_eq!(rows.len(), 1, "expected one captured row");
        assert_eq!(
            rows[0].channel,
            LogSource::Ptu,
            "captured row must record the real channel of the source file",
        );
    }

    // ─── Inference wiring ──────────────────────────────────────────────

    /// Two log lines that, together, exercise the
    /// `implicit_death_after_vehicle_destruction` built-in inference
    /// rule: a vehicle destruction followed (within 15s) by a spawn
    /// resolution. The Vehicle Destruction regex demands the shape
    /// `vehicle '<class>' [<id>] ... destroyLevel <n> ... caused by '<c>'`
    /// — lowercase `vehicle`, `destroyLevel` as a single token.
    const VEH_DESTRUCTION_LINE: &str = "<2026-05-17T14:02:30.000Z> [Notice] <Vehicle Destruction> CVehicle::OnAdvanceDestroyLevel: vehicle 'AEGS_Cutlass_Black_4321' [4321] destroyLevel 2 caused by 'self' zone 'OOC_Stanton_2b_Daymar' [Team_ActorTech][Vehicle]";
    const RESOLVE_SPAWN_LINE: &str = "<2026-05-17T14:02:35.000Z> [Notice] <ResolveSpawnLocation Location Not Found> Could not resolve initial spawn location from spawning module for player id: [9794883988961], setting spawn zone location zonehost to solar system fallback [Team_BackendServices][Services]";

    fn run_two_line_drain(
        storage: &Storage,
        rules: &RuleCache,
        log_source: &str,
        log_source_enum: LogSource,
        flag_on: bool,
        window: &mut InferenceWindow,
        buffer: &[(String, u64)],
    ) {
        let stats = parking_lot::Mutex::new(TailStats::default());
        let rules_snapshot = rules.snapshot();
        let inference_rules = rules.combined_inference_rules();
        let burst_rules = builtin_burst_rules();
        process_buffer(
            buffer,
            storage,
            &stats,
            log_source,
            log_source_enum,
            None,
            &rules_snapshot,
            &inference_rules,
            &burst_rules,
            flag_on,
            "",
            window,
        );
    }

    #[tokio::test]
    async fn drain_rereads_from_head_after_rotation_that_outgrew_offset() {
        // F2 regression: the launcher replaces Game.log with a fresh
        // session that has ALREADY grown past our saved offset. The old
        // `len < offset` check saw `len >= offset` and seeked mid-file,
        // skipping the new session's opening lines. The signature-based
        // reset must re-read from the head instead.
        let dir = TempDir::new().expect("tempdir");
        let log_path = dir.path().join("Game.log");
        let path_str = log_path.to_string_lossy().to_string();
        let storage = Storage::open(&dir.path().join("test.db")).expect("open storage");
        let rules = RuleCache::new();
        let stats = parking_lot::Mutex::new(TailStats::default());
        let mut window = InferenceWindow::default();
        let mut offset = 0u64;
        let mut last_sig: Option<String> = None;

        // Session 1: benign filler, large enough to be signable
        // (>= FILE_SIG_HEAD_BYTES) but classifying to nothing.
        let s1: String = (0..20)
            .map(|i| format!("session-one filler line number {i}\n"))
            .collect();
        tokio::fs::write(&log_path, s1.as_bytes())
            .await
            .expect("write s1");
        let s1_len = tokio::fs::metadata(&log_path).await.unwrap().len();
        assert!(
            s1_len >= FILE_SIG_HEAD_BYTES as u64,
            "test setup: session 1 must be signable ({s1_len} < {FILE_SIG_HEAD_BYTES})"
        );
        drain(
            &log_path,
            &path_str,
            &mut offset,
            &mut last_sig,
            &storage,
            &stats,
            &rules,
            false,
            "",
            &mut window,
        )
        .await
        .expect("drain s1");
        assert_eq!(offset, s1_len);
        let sig1 = last_sig.clone();
        assert!(sig1.is_some(), "session 1 must produce a signature");

        // Rotation: a fresh, longer log whose opening event lives inside
        // the first `s1_len` bytes a buggy mid-file seek would jump over.
        // The replacement changes the head bytes (and the OS may also
        // reset the creation time), so the signature differs and the
        // reset fires — the head component is what defends the case where
        // only the content differs.
        let s2_filler: String = (0..12)
            .map(|i| format!("session-two filler line number {i}\n"))
            .collect();
        let s2 = format!("{VEH_DESTRUCTION_LINE}\n{RESOLVE_SPAWN_LINE}\n{s2_filler}");
        tokio::fs::write(&log_path, s2.as_bytes())
            .await
            .expect("write s2");
        let s2_len = tokio::fs::metadata(&log_path).await.unwrap().len();
        assert!(
            s2_len > s1_len,
            "test setup: session 2 must outgrow session 1's offset ({s2_len} <= {s1_len})"
        );

        drain(
            &log_path,
            &path_str,
            &mut offset,
            &mut last_sig,
            &storage,
            &stats,
            &rules,
            false,
            "",
            &mut window,
        )
        .await
        .expect("drain s2");

        assert_ne!(last_sig, sig1, "signature must track the replacement file");
        let rows = storage.recent_events(20).expect("recent_events");
        assert!(
            rows.iter().any(|r| r.event_type == "vehicle_destruction"),
            "rotated session's opening event must be re-read from the head, not skipped; got {:?}",
            rows.iter().map(|r| &r.event_type).collect::<Vec<_>>()
        );

        // The persisted cursor reflects the reset + new signature so a
        // tray restart mid-session resumes correctly.
        let (stored_off, stored_sig) = storage.read_tail_cursor(&path_str).expect("read cursor");
        assert_eq!(stored_off, s2_len);
        assert_eq!(stored_sig, last_sig);
    }

    #[test]
    fn inference_window_emits_inferred_event_when_rule_matches() {
        // Flag on + the VehicleDestruction → ResolveSpawn pair triggers
        // the built-in death-after-destruction rule, so the events
        // table picks up a synthetic `player_death` row alongside the
        // two observed events. End-to-end check that infer() is wired.
        let (_dir, storage) = open_temp_storage();
        let rules = RuleCache::new();
        let mut window = InferenceWindow::default();
        let buffer = vec![
            (VEH_DESTRUCTION_LINE.to_string(), 0),
            (RESOLVE_SPAWN_LINE.to_string(), 100),
        ];
        run_two_line_drain(
            &storage,
            &rules,
            "live",
            LogSource::Live,
            true,
            &mut window,
            &buffer,
        );

        let rows = storage.recent_events(10).expect("recent_events");
        let inferred_deaths = rows
            .iter()
            .filter(|r| r.event_type == "player_death")
            .count();
        assert_eq!(
            inferred_deaths,
            1,
            "expected exactly one inferred player_death; rows = {:?}",
            rows.iter().map(|r| &r.event_type).collect::<Vec<_>>()
        );
        let inferred = rows
            .iter()
            .find(|r| r.event_type == "player_death")
            .expect("inferred row present");
        assert!(
            inferred.raw_line.starts_with("inferred:"),
            "inferred row's raw_line must self-identify; got {:?}",
            inferred.raw_line
        );
    }

    #[test]
    fn inference_window_does_not_double_emit_on_overlapping_windows() {
        // Run the same drain twice. The second pass replays the same
        // trigger envelope through the window, but `emitted` (in-memory)
        // plus `ON CONFLICT DO NOTHING` (storage) must collapse both
        // attempts into one row.
        let (_dir, storage) = open_temp_storage();
        let rules = RuleCache::new();
        let mut window = InferenceWindow::default();
        let buffer = vec![
            (VEH_DESTRUCTION_LINE.to_string(), 0),
            (RESOLVE_SPAWN_LINE.to_string(), 100),
        ];
        run_two_line_drain(
            &storage,
            &rules,
            "live",
            LogSource::Live,
            true,
            &mut window,
            &buffer,
        );
        run_two_line_drain(
            &storage,
            &rules,
            "live",
            LogSource::Live,
            true,
            &mut window,
            &buffer,
        );

        let rows = storage.recent_events(20).expect("recent_events");
        let inferred_deaths = rows
            .iter()
            .filter(|r| r.event_type == "player_death")
            .count();
        assert_eq!(
            inferred_deaths, 1,
            "overlapping window must not double-emit the same inferred event",
        );
    }

    #[test]
    fn inference_window_disabled_when_flag_off() {
        // Same trigger pair, but with the v2 flag off — observed
        // events still land, the inferred row must not. This is the
        // promise to flag-off installs that the v2 surface stays
        // dormant.
        let (_dir, storage) = open_temp_storage();
        let rules = RuleCache::new();
        let mut window = InferenceWindow::default();
        let buffer = vec![
            (VEH_DESTRUCTION_LINE.to_string(), 0),
            (RESOLVE_SPAWN_LINE.to_string(), 100),
        ];
        run_two_line_drain(
            &storage,
            &rules,
            "live",
            LogSource::Live,
            false,
            &mut window,
            &buffer,
        );

        let rows = storage.recent_events(10).expect("recent_events");
        let inferred_deaths = rows
            .iter()
            .filter(|r| r.event_type == "player_death")
            .count();
        assert_eq!(
            inferred_deaths, 0,
            "flag-off install must not emit any inferred events",
        );
    }

    // ─── Loadout burst regression ──────────────────────────────────────

    /// Three `<AttachmentReceived> [Inventory]` lines at tight offsets
    /// (0, 1, 2) form a loadout burst (min_burst_size = 3, max_member_gap
    /// = 1).  The burst summary must carry `kind = "loadout_restore"` and
    /// a non-empty `categories` map so the web loadout widget can render
    /// it.
    ///
    /// Regression for the bug where the rule_id check used the bare string
    /// `"loadout_restore"` instead of `"loadout_restore_burst"`, which
    /// meant the condition was never true and every burst summary was
    /// emitted with `kind = null` / `categories = null`.
    #[test]
    fn loadout_burst_summary_carries_kind_and_categories() {
        // Three realistic AttachmentReceived lines spanning two item
        // categories (undersuit/armor + weapon) so that `categories` has
        // at least two keys.
        const LINE_UNDERSUIT: &str = concat!(
            "<2026-05-03T17:52:57.219Z> [Notice] <AttachmentReceived> ",
            "Player[TheCodeSaiyan] ",
            "Attachment[rsi_odyssey_undersuit_01_01_01_200000000232, ",
            "rsi_odyssey_undersuit_01_01_01, 200000000232] ",
            "Status[persistent] Port[Armor_Undersuit] Elapsed[27.480394] ",
            "[Team_CoreGameplayFeatures][Inventory]"
        );
        const LINE_HELMET: &str = concat!(
            "<2026-05-03T17:52:57.220Z> [Notice] <AttachmentReceived> ",
            "Player[TheCodeSaiyan] ",
            "Attachment[rsi_odyssey_helmet_01_01_01_200000000233, ",
            "rsi_odyssey_helmet_01_01_01, 200000000233] ",
            "Status[persistent] Port[Armor_Helmet] Elapsed[27.480500] ",
            "[Team_CoreGameplayFeatures][Inventory]"
        );
        const LINE_PISTOL: &str = concat!(
            "<2026-05-03T17:52:57.221Z> [Notice] <AttachmentReceived> ",
            "Player[TheCodeSaiyan] ",
            "Attachment[behr_pistol_ballistic_01_200000000234, ",
            "behr_pistol_ballistic_01, 200000000234] ",
            "Status[persistent] Port[WeaponRight] Elapsed[27.480600] ",
            "[Team_CoreGameplayFeatures][Inventory]"
        );

        let (_dir, storage) = open_temp_storage();
        let rules = RuleCache::new();
        let mut window = InferenceWindow::default();
        let buffer = vec![
            (LINE_UNDERSUIT.to_string(), 0u64),
            (LINE_HELMET.to_string(), 1u64),
            (LINE_PISTOL.to_string(), 2u64),
        ];
        run_two_line_drain(
            &storage,
            &rules,
            "live",
            LogSource::Live,
            false,
            &mut window,
            &buffer,
        );

        let rows = storage.recent_events(20).expect("recent_events");
        let burst_row = rows
            .iter()
            .find(|r| r.event_type == "burst_summary")
            .expect(
                "expected a burst_summary row — the loadout burst must have been stored; \
                 if missing, process_buffer may not be forming the burst",
            );

        let payload: serde_json::Value =
            serde_json::from_str(&burst_row.payload_json).expect("payload must be valid JSON");

        assert_eq!(
            payload.get("kind").and_then(|v| v.as_str()),
            Some("loadout_restore"),
            "burst_summary kind must be 'loadout_restore'; \
             got payload = {payload}  \
             (regression: old code checked rule_id == \"loadout_restore\" instead of \
             \"loadout_restore_burst\", leaving kind = null forever)",
        );

        let categories = payload
            .get("categories")
            .expect("burst_summary must have a 'categories' field");
        assert!(
            categories.is_object() && !categories.as_object().unwrap().is_empty(),
            "burst_summary categories must be a non-empty object; got {categories}",
        );

        // TDD: items must be present, one entry per input line, each with a
        // non-empty port and a non-empty category.
        let items = payload
            .get("items")
            .expect("burst_summary must have an 'items' field")
            .as_array()
            .expect("items must be a JSON array");
        assert_eq!(
            items.len(),
            3,
            "items must have one entry per AttachmentReceived member; got {items:?}",
        );
        for item in items {
            let port = item
                .get("port")
                .and_then(|v| v.as_str())
                .expect("each item must have a non-empty port string");
            assert!(
                !port.is_empty(),
                "item port must be non-empty; got {item:?}"
            );
            let cat = item
                .get("category")
                .and_then(|v| v.as_str())
                .expect("each item must have a category string");
            assert!(
                !cat.is_empty(),
                "item category must be non-empty; got {item:?}"
            );
        }
        // Spot-check: helmet item has the expected port.
        assert!(
            items
                .iter()
                .any(|it| it.get("port").and_then(|v| v.as_str()) == Some("Armor_Helmet")),
            "expected one item with port == 'Armor_Helmet'; got {items:?}",
        );
    }
}
