//! Background worker that drains locally-stored events to the
//! StarStats API.
//!
//! Loop:
//! 1. Read `sync_cursor.last_event_id`.
//! 2. Read up to `batch_size` events with `id > cursor`.
//! 3. POST as an [`IngestBatch`] with the configured bearer token.
//! 4. On success, advance the cursor to the highest id in the batch.
//! 5. Sleep `interval_secs` and repeat.
//!
//! Failures (network, 5xx) are logged and retried after the sleep —
//! the cursor is only advanced on a 2xx response, so events never get
//! lost.
//!
//! Auth invalidation (401/403) is treated specially: the worker clears
//! the persisted device token, flips `AccountStatus::auth_lost`, and
//! stops attempting upstream drains until the user re-pairs. The tail
//! loop keeps appending events to local SQLite throughout.

use crate::config;
use crate::state::AccountStatus;
use crate::storage::{Storage, UnsentEvent};
use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use starstats_core::location_catalog::LocationCatalog;
use starstats_core::location_classifier::{classify, ResolvedLocation};
use starstats_core::wire::{EventEnvelope, IngestBatch, LogSource, SourceRange};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::sync::Notify;

#[derive(Debug, Deserialize)]
struct IngestResponse {
    #[allow(dead_code)]
    batch_id: String,
    accepted: u32,
    duplicate: u32,
    rejected: u32,
}

/// Shape returned by `GET /v1/auth/me`. Mirrors the server's
/// `auth_routes::MeResponse` — duplicated rather than depending on
/// the server crate to keep the tray's compile graph small.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeResponse {
    pub user_id: String,
    pub email: String,
    pub claimed_handle: String,
    pub email_verified: bool,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct SyncStats {
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub batches_sent: u64,
    pub events_accepted: u64,
    pub events_duplicate: u64,
    pub events_rejected: u64,
    /// Events the poison-pill isolation path shelved after the server
    /// returned 4xx (other than 401/403) on a single-event retry.
    /// Counted lifetime-of-process; the persistent count lives on
    /// rows whose `sent_at` starts with `__quarantined_` and can be
    /// queried via `storage.count_quarantined()`.
    pub events_quarantined: u64,
}

/// Abort the currently-running sync worker (if any) and spawn a
/// fresh one with the current persisted config. Used by `save_config`
/// and `redeem_pair` to pick up new tokens / endpoints / enabled-flag
/// values without requiring an app restart.
///
/// Idempotent: calling it when the new config also fails to spawn a
/// worker (e.g. `enabled = false`) just leaves `sync_handle` as
/// `None`, which is the same state the boot path would produce.
///
/// Reads config from disk so the caller doesn't have to thread the
/// fresh config in — there's exactly one place that mutates it
/// (`config::save`), and it's always called before this helper.
pub fn respawn(
    storage: Arc<crate::storage::Storage>,
    sync_stats: Arc<parking_lot::Mutex<SyncStats>>,
    account_status: Arc<parking_lot::Mutex<crate::state::AccountStatus>>,
    sync_kick: Arc<SyncKick>,
    sync_handle: Arc<parking_lot::Mutex<SyncHandles>>,
    app_handle: tauri::AppHandle,
    location_catalog: Arc<parking_lot::RwLock<Arc<LocationCatalog>>>,
) {
    // Abort first so the old workers stop draining with stale auth
    // before we spawn fresh ones. `abort()` is non-blocking; the
    // tokio runtime cleans up at the next poll.
    {
        let mut guard = sync_handle.lock();
        if guard.is_running() {
            guard.abort();
            tracing::info!("sync: aborted previous worker(s)");
        }
    }

    let cfg = match crate::config::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "sync respawn: config load failed; leaving worker stopped");
            return;
        }
    };

    let handles = start(
        cfg.remote_sync.clone(),
        cfg.sync_with_cloud,
        storage,
        sync_stats,
        account_status,
        sync_kick,
        app_handle,
        location_catalog,
    );

    if handles.is_running() {
        tracing::info!(
            enabled = cfg.remote_sync.enabled,
            has_api_url = cfg.remote_sync.api_url.is_some(),
            has_access_token = cfg.remote_sync.access_token.is_some(),
            has_claimed_handle = cfg.remote_sync.claimed_handle.is_some(),
            bulk_running = handles.bulk.is_some(),
            priority_running = handles.priority.is_some(),
            "sync: spawned fresh worker(s)"
        );
    } else {
        // Include the same config-presence fields here so a "no worker
        // running" log line directly names whichever field is missing
        // (most often access_token after a token clear).
        tracing::info!(
            enabled = cfg.remote_sync.enabled,
            has_api_url = cfg.remote_sync.api_url.is_some(),
            has_access_token = cfg.remote_sync.access_token.is_some(),
            has_claimed_handle = cfg.remote_sync.claimed_handle.is_some(),
            "sync: config incomplete or disabled; no worker running"
        );
    }
    *sync_handle.lock() = handles;
}

/// Two independent kick channels, one per sync lane, so "Sync now" and
/// config-change nudges wake BOTH lanes. History (M-T1): a single shared
/// `Arc<Notify>` woke only one lane under `notify_one()`; switching to
/// `notify_waiters()` would wake both currently-waiting lanes but drop a kick
/// that arrives while a lane is mid-drain (it stores no permit). Per-lane
/// `notify_one()` stores a permit per lane, so a kick during a drain is
/// honored on that lane's next wait instead of being lost.
#[derive(Default)]
pub struct SyncKick {
    priority: Notify,
    bulk: Notify,
}

impl SyncKick {
    /// Wake both sync lanes (priority + bulk).
    pub fn kick_all(&self) {
        self.priority.notify_one();
        self.bulk.notify_one();
    }

    /// The kick channel a given lane awaits.
    fn for_lane(&self, lane: Lane) -> &Notify {
        match lane {
            Lane::Priority => &self.priority,
            Lane::Bulk => &self.bulk,
        }
    }
}

#[cfg(test)]
mod sync_kick_tests {
    use super::*;

    #[tokio::test]
    async fn kick_all_stores_a_permit_for_both_lanes() {
        let k = SyncKick::default();
        k.kick_all();
        // Both permits are stored up-front (the mid-drain property a shared
        // notify_waiters() would lack), so each lane's notified() resolves
        // without a pre-registered waiter.
        for lane in [Lane::Bulk, Lane::Priority] {
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                k.for_lane(lane).notified(),
            )
            .await
            .expect("kick_all must store a permit for each lane");
        }
    }
}

/// Which lane is calling `drain_lane`. Drives the filter passed to
/// `Storage::read_unsent_filtered` and gates the idle-heartbeat
/// behaviour (only the bulk lane heartbeats — the priority lane wakes
/// often enough on its own that an idle heartbeat would just spam
/// `/v1/auth/me`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Priority,
    Bulk,
}

impl Lane {
    fn label(self) -> &'static str {
        match self {
            Lane::Priority => "priority",
            Lane::Bulk => "bulk",
        }
    }
}

/// Spawn the sync workers. Returns up to TWO `JoinHandle`s — one for
/// the priority lane (if `priority_event_types` is non-empty), one
/// for the bulk lane. `None`s in the returned tuple mean that lane
/// isn't running. Callers store both handles so `respawn` can abort
/// them atomically when config changes.
///
/// `kick` lets the UI cut short the post-drain sleep on demand —
/// notifying it wakes BOTH lanes so a manual "Sync now" doesn't
/// leave one lane snoozing.
#[allow(clippy::too_many_arguments)]
pub fn start(
    cfg: config::RemoteSyncConfig,
    sync_with_cloud: bool,
    storage: Arc<Storage>,
    sync_stats: Arc<parking_lot::Mutex<SyncStats>>,
    account_status: Arc<parking_lot::Mutex<AccountStatus>>,
    kick: Arc<SyncKick>,
    app_handle: tauri::AppHandle,
    location_catalog: Arc<parking_lot::RwLock<Arc<LocationCatalog>>>,
) -> SyncHandles {
    if !cfg.enabled {
        return SyncHandles::default();
    }
    let Some(api_url) = cfg.api_url.clone() else {
        return SyncHandles::default();
    };
    let Some(claimed_handle) = cfg.claimed_handle.clone() else {
        return SyncHandles::default();
    };
    let Some(access_token) = cfg.access_token.clone() else {
        return SyncHandles::default();
    };

    let bulk_interval = Duration::from_secs(cfg.interval_secs.max(5));
    let priority_interval = Duration::from_secs(cfg.priority_interval_secs.max(1));
    let tuning = DrainTuning::from_config(&cfg);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(_) => return SyncHandles::default(),
    };

    let priority_types = cfg.priority_event_types.clone();
    let has_priority = !priority_types.is_empty();

    let bulk_handle = spawn_lane(
        Lane::Bulk,
        bulk_interval,
        client.clone(),
        api_url.clone(),
        access_token.clone(),
        claimed_handle.clone(),
        storage.clone(),
        sync_stats.clone(),
        account_status.clone(),
        kick.clone(),
        priority_types.clone(),
        tuning,
        sync_with_cloud,
        app_handle.clone(),
        location_catalog.clone(),
    );

    let priority_handle = if has_priority {
        Some(spawn_lane(
            Lane::Priority,
            priority_interval,
            client,
            api_url,
            access_token,
            claimed_handle,
            storage,
            sync_stats,
            account_status,
            kick,
            priority_types,
            tuning,
            false, // priority lane never piggybacks preferences
            app_handle,
            location_catalog,
        ))
    } else {
        None
    };

    SyncHandles {
        bulk: Some(bulk_handle),
        priority: priority_handle,
    }
}

/// Pair of sync worker handles. `bulk` always runs when sync is
/// enabled + paired; `priority` only runs when the user has at least
/// one event type in their priority list. `Default` is "no workers
/// running" — used as the return when sync is disabled or the config
/// is incomplete.
#[derive(Default)]
pub struct SyncHandles {
    pub bulk: Option<tauri::async_runtime::JoinHandle<()>>,
    pub priority: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl SyncHandles {
    /// Abort both lanes. Idempotent; safe to call when either or
    /// both handles are absent.
    pub fn abort(&mut self) {
        if let Some(h) = self.bulk.take() {
            h.abort();
        }
        if let Some(h) = self.priority.take() {
            h.abort();
        }
    }

    /// True if at least one lane is running.
    pub fn is_running(&self) -> bool {
        self.bulk.is_some() || self.priority.is_some()
    }
}

/// Ceiling on the failure backoff — a down server is re-probed at most once
/// every 5 minutes rather than every lane interval.
const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// Gap between back-to-back catch-up drains while Star Citizen is NOT
/// running. Effectively "immediately, but yield the runtime" — a
/// six-figure backlog is then bounded by server round-trip time rather
/// than by a local timer.
const CATCH_UP_DELAY_IDLE: Duration = Duration::from_millis(250);

/// Gap between back-to-back catch-up drains while the game IS running.
/// Still ~30x faster than the default 60 s bulk interval, but paced so
/// the uplink never contends with the session for CPU or bandwidth.
/// Pairs with the `batch_size` (not `catch_up_batch_size`) page cap
/// applied in-game.
const CATCH_UP_DELAY_IN_GAME: Duration = Duration::from_secs(2);

/// How long a [`GameProbe`] answer is reused before re-scanning the
/// process table. `process_guard::is_starcitizen_running` builds and
/// refreshes a whole `System` snapshot (tens of milliseconds) — fine
/// once a minute, far too expensive once per 250 ms catch-up tick.
const PROCESS_PROBE_TTL: Duration = Duration::from_secs(15);

/// Per-lane drain sizing, resolved once at spawn from the persisted
/// [`config::RemoteSyncConfig`]. `Copy` so it threads through
/// `spawn_lane` without cloning.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrainTuning {
    /// Steady-state page size — tuned for latency.
    batch_size: usize,
    /// Backlog page size — tuned for throughput. Always >= `batch_size`.
    catch_up_batch_size: usize,
    /// Ceiling on the estimated JSON body size of one `/v1/ingest` POST.
    max_batch_bytes: usize,
    /// Master switch for burst catch-up. False restores the strict
    /// one-page-per-interval cadence.
    catch_up_enabled: bool,
}

impl DrainTuning {
    pub(crate) fn from_config(cfg: &config::RemoteSyncConfig) -> Self {
        let batch_size = cfg.batch_size.max(1);
        Self {
            batch_size,
            // A catch-up page smaller than the steady-state page would
            // make backlogs drain SLOWER than normal traffic; clamp up.
            catch_up_batch_size: cfg.catch_up_batch_size.max(batch_size),
            // 64 KiB floor: a byte cap below one envelope would chunk
            // every batch down to a single event and crawl.
            max_batch_bytes: cfg.max_batch_bytes.max(64 * 1024),
            catch_up_enabled: cfg.catch_up_enabled,
        }
    }

    /// The page size to read for this tick. Catch-up only escalates
    /// while the game is DOWN — an in-game backlog drain stays on the
    /// steady-state page so the uplink never competes with the session.
    /// The steady-state page size, after clamping. The backlog
    /// readout compares queue depth against this to decide whether the
    /// worker is (or is about to be) in catch-up.
    pub(crate) fn steady_page(&self) -> usize {
        self.batch_size
    }

    pub(crate) fn page_size(&self, catching_up: bool, game_running: bool) -> usize {
        if catching_up && self.catch_up_enabled && !game_running {
            self.catch_up_batch_size
        } else {
            self.batch_size
        }
    }

    /// How long to wait before the next drain. A lane that just shipped
    /// a full page (`catching_up`) with no failures loops almost
    /// immediately instead of sleeping the configured interval — that
    /// single change is what turns a 300k-event backlog from days into
    /// minutes.
    pub(crate) fn delay(
        &self,
        catching_up: bool,
        game_running: bool,
        interval: Duration,
        consecutive_failures: u32,
    ) -> Duration {
        if catching_up && self.catch_up_enabled && consecutive_failures == 0 {
            return if game_running {
                CATCH_UP_DELAY_IN_GAME
            } else {
                CATCH_UP_DELAY_IDLE
            };
        }
        backoff_delay(interval, consecutive_failures)
    }
}

#[cfg(test)]
mod drain_tuning_tests {
    use super::*;

    fn cfg(batch: usize, catch_up: usize, bytes: usize, enabled: bool) -> config::RemoteSyncConfig {
        config::RemoteSyncConfig {
            batch_size: batch,
            catch_up_batch_size: catch_up,
            max_batch_bytes: bytes,
            catch_up_enabled: enabled,
            ..config::RemoteSyncConfig::default()
        }
    }

    #[test]
    fn catch_up_page_is_never_smaller_than_the_steady_state_page() {
        // A user who typed a big steady-state batch_size must not get a
        // SLOWER drain while backlogged.
        let t = DrainTuning::from_config(&cfg(1000, 200, 3 << 20, true));
        assert_eq!(t.catch_up_batch_size, 1000);
        assert_eq!(t.page_size(true, false), 1000);
    }

    #[test]
    fn byte_cap_has_a_floor_so_a_zero_never_stalls_the_drain() {
        let t = DrainTuning::from_config(&cfg(200, 2000, 0, true));
        assert_eq!(t.max_batch_bytes, 64 * 1024);
    }

    #[test]
    fn page_escalates_only_while_backlogged_and_out_of_game() {
        let t = DrainTuning::from_config(&cfg(200, 2000, 3 << 20, true));
        assert_eq!(t.page_size(false, false), 200, "idle queue → steady page");
        assert_eq!(t.page_size(true, true), 200, "in-game → steady page");
        assert_eq!(
            t.page_size(true, false),
            2000,
            "backlog + game down → burst page"
        );
    }

    #[test]
    fn disabling_catch_up_restores_one_page_per_interval() {
        let t = DrainTuning::from_config(&cfg(200, 2000, 3 << 20, false));
        assert_eq!(t.page_size(true, false), 200);
        assert_eq!(
            t.delay(true, false, Duration::from_secs(60), 0),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn catch_up_delay_collapses_the_interval_but_yields_to_the_game() {
        let t = DrainTuning::from_config(&cfg(200, 2000, 3 << 20, true));
        let interval = Duration::from_secs(60);
        assert_eq!(
            t.delay(false, false, interval, 0),
            interval,
            "empty queue → configured interval"
        );
        assert_eq!(t.delay(true, false, interval, 0), CATCH_UP_DELAY_IDLE);
        assert_eq!(t.delay(true, true, interval, 0), CATCH_UP_DELAY_IN_GAME);
        // Failures always win over catch-up: a 4xx/5xx storm must back
        // off, not hot-loop at 250 ms.
        assert_eq!(
            t.delay(true, false, interval, 3),
            backoff_delay(interval, 3)
        );
    }
}

/// TTL-cached "is Star Citizen running?" probe.
///
/// Sync is NOT gated on the game — the drain runs whether or not Star
/// Citizen is up, and always has. The probe only decides how AGGRESSIVE
/// a backlog drain may be: full burst when the game is down, paced when
/// it is up. A stale answer is harmless — worst case is one catch-up
/// tick at the wrong cadence.
struct GameProbe {
    cached: Option<(Instant, bool)>,
}

impl GameProbe {
    fn new() -> Self {
        Self { cached: None }
    }

    fn is_running(&mut self) -> bool {
        let now = Instant::now();
        if let Some((at, answer)) = self.cached {
            if now.duration_since(at) < PROCESS_PROBE_TTL {
                return answer;
            }
        }
        let answer = crate::process_guard::is_starcitizen_running();
        self.cached = Some((now, answer));
        answer
    }
}

/// What one `drain_lane` pass learned about the queue behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DrainOutcome {
    /// The page came back FULL, so there is almost certainly more
    /// behind it. Drives catch-up: the lane loops again immediately
    /// instead of sleeping its interval. Inferred from the page rather
    /// than a `COUNT(*)` so the hot path stays a single query.
    page_was_full: bool,
}

/// Rough serialized size of one envelope, used to keep a POST body
/// under [`DrainTuning::max_batch_bytes`].
///
/// Deliberately an OVER-estimate: `raw_line` and `payload_json` ship
/// verbatim, and the constant covers the envelope scaffolding plus the
/// `resolved_location` block that `build_batch` stamps on afterwards
/// (not visible from the stored row). Under-counting costs a 413 and a
/// wasted multi-megabyte upload; over-counting costs one extra request.
fn estimated_envelope_bytes(e: &UnsentEvent) -> usize {
    const ENVELOPE_OVERHEAD: usize = 384;
    e.idempotency_key.len()
        + e.raw_line.len()
        + e.payload_json.len()
        + e.log_source.len()
        + ENVELOPE_OVERHEAD
}

/// Split a page into chunks whose estimated body size stays under
/// `max_bytes`, preserving ascending-id order both within and across
/// chunks.
///
/// This is what makes a large `catch_up_batch_size` safe: the page size
/// controls how much SQLite work one tick does, and this controls how
/// much of it fits in one HTTP request. Without it a 2000-event page of
/// long raw lines would 413 and force the poison-pill path to bisect
/// its way down — correct, but it re-uploads megabytes on each split.
///
/// An event that alone exceeds `max_bytes` still gets its own chunk:
/// dropping it locally would lose data, so it ships and the server
/// decides.
fn chunk_by_bytes(events: Vec<UnsentEvent>, max_bytes: usize) -> Vec<Vec<UnsentEvent>> {
    let mut chunks: Vec<Vec<UnsentEvent>> = Vec::new();
    let mut current: Vec<UnsentEvent> = Vec::new();
    let mut current_bytes = 0usize;
    for e in events {
        let size = estimated_envelope_bytes(&e);
        if !current.is_empty() && current_bytes + size > max_bytes {
            chunks.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes += size;
        current.push(e);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod chunking_tests {
    use super::*;

    fn ev(id: i64, raw_len: usize) -> UnsentEvent {
        UnsentEvent {
            id,
            idempotency_key: format!("key-{id}"),
            payload_json: "{}".to_string(),
            raw_line: "x".repeat(raw_len),
            log_source: "live".to_string(),
            source_offset: id as u64,
        }
    }

    #[test]
    fn a_small_page_stays_one_chunk() {
        let chunks = chunk_by_bytes((1..=50).map(|i| ev(i, 200)).collect(), 3 << 20);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 50);
    }

    #[test]
    fn a_large_page_splits_and_every_chunk_fits_the_cap() {
        let cap = 64 * 1024;
        // 2000 envelopes at ~1 KB each is ~2 MB, well over a 64 KiB cap.
        let chunks = chunk_by_bytes((1..=2000).map(|i| ev(i, 640)).collect(), cap);
        assert!(chunks.len() > 1, "must split");
        for c in &chunks {
            let bytes: usize = c.iter().map(estimated_envelope_bytes).sum();
            assert!(bytes <= cap, "chunk of {} events is {bytes} B", c.len());
        }
    }

    #[test]
    fn chunking_preserves_every_event_in_ascending_id_order() {
        let chunks = chunk_by_bytes((1..=500).map(|i| ev(i, 900)).collect(), 100 * 1024);
        let ids: Vec<i64> = chunks.iter().flatten().map(|e| e.id).collect();
        assert_eq!(ids, (1..=500).collect::<Vec<i64>>());
    }

    #[test]
    fn an_event_bigger_than_the_cap_is_sent_alone_not_dropped() {
        // Losing local data to a byte cap would be far worse than a
        // server-side rejection, so the oversized row still ships.
        let events = vec![ev(1, 10), ev(2, 200_000), ev(3, 10)];
        let chunks = chunk_by_bytes(events, 1024);
        let ids: Vec<i64> = chunks.iter().flatten().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        let oversized = chunks.iter().find(|c| c.iter().any(|e| e.id == 2)).unwrap();
        assert_eq!(oversized.len(), 1, "oversized row must not drag neighbours");
    }

    #[test]
    fn an_empty_page_produces_no_chunks() {
        assert!(chunk_by_bytes(Vec::new(), 3 << 20).is_empty());
    }
}

/// Floor between bulk-lane idle heartbeats (`GET /v1/auth/me`). Independent of
/// the lane interval so a low `interval_secs` (5s floor) doesn't heartbeat on
/// every empty drain. 60s matches the default bulk cadence, so default
/// behaviour is unchanged; only sub-60s intervals get throttled.
const HEARTBEAT_MIN_INTERVAL: Duration = Duration::from_secs(60);
/// How often the opt-in unknown-tag report is pushed. Tag sightings change
/// on the timescale of game patches, so this is deliberately slow — it is
/// diagnostic metadata, not telemetry.
const TAG_REPORT_MIN_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// Same conservative charset the SERVER enforces. Duplicated on purpose:
/// this check protects the user's stated intent (tags only, never log
/// bodies) before anything leaves the machine, and the server's protects
/// the server from a modified client. Neither is redundant.
fn tag_is_reportable(s: &str) -> bool {
    if s.is_empty() || s.len() > 200 {
        return false;
    }
    if s.chars().any(|c| {
        c.is_control()
            || matches!(
                c,
                '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | '\\' | '|'
            )
    }) {
        return false;
    }
    s.chars().any(|c| c.is_ascii_alphanumeric())
}

fn tag_report_due(last: Option<Instant>, now: Instant) -> bool {
    match last {
        None => true,
        Some(t) => now.duration_since(t) >= TAG_REPORT_MIN_INTERVAL,
    }
}

/// Whether the bulk lane's idle heartbeat is due, given when it last fired.
/// Pure so the rate-limit is unit-tested. `None` (never fired) is always due.
fn heartbeat_due(last: Option<Instant>, now: Instant) -> bool {
    match last {
        None => true,
        Some(t) => now.duration_since(t) >= HEARTBEAT_MIN_INTERVAL,
    }
}

/// Whether a 401/403 ingest rejection means "sync is disabled for this uplink"
/// (the device is still validly paired — `sync_enabled` is off server-side)
/// rather than a revoked/invalid token. The former MUST NOT clear the device
/// credentials — doing so unpaired every freshly-paired device in a loop
/// (`sync_enabled` defaults false on pair). Pure so the distinction is tested.
fn is_sync_disabled_rejection(body: &str) -> bool {
    body.contains("device_sync_disabled")
}

#[cfg(test)]
mod ingest_auth_tests {
    use super::*;

    #[test]
    fn sync_disabled_body_is_not_treated_as_auth_loss() {
        let disabled = r#"{"error":"device_sync_disabled","detail":"this uplink's sync is disabled — re-enable from the Connected Uplinks page"}"#;
        assert!(is_sync_disabled_rejection(disabled));
        // A genuinely revoked / invalid token must still clear (returns false).
        assert!(!is_sync_disabled_rejection(
            r#"{"error":"invalid_token","detail":"signature verification failed"}"#
        ));
        assert!(!is_sync_disabled_rejection(r#"{"error":"unauthorized"}"#));
        assert!(!is_sync_disabled_rejection(""));
    }
}

#[cfg(test)]
mod heartbeat_tests {
    use super::*;

    #[test]
    fn tag_report_respects_its_own_floor() {
        // Move the OBSERVER forward rather than the sent-at backward.
        // `Instant` is monotonic from boot, so `now - 6h` underflows to
        // `None` on any machine with under six hours of uptime — which is
        // every CI runner. This test failed on Windows CI while passing
        // locally for exactly that reason; `checked_add` cannot underflow.
        // The sibling heartbeat test gets away with `checked_sub` only
        // because its interval is 60 seconds.
        let sent_at = Instant::now();
        assert!(tag_report_due(None, sent_at), "never sent → due");
        assert!(
            !tag_report_due(Some(sent_at), sent_at),
            "just sent → not due"
        );

        let halfway = sent_at.checked_add(TAG_REPORT_MIN_INTERVAL / 2).unwrap();
        assert!(!tag_report_due(Some(sent_at), halfway), "inside the floor");

        let elapsed = sent_at.checked_add(TAG_REPORT_MIN_INTERVAL).unwrap();
        assert!(tag_report_due(Some(sent_at), elapsed), "floor reached");
    }

    #[test]
    fn only_engine_symbol_tags_are_reportable() {
        // The privacy contract, enforced before anything leaves the machine.
        for good in [
            "LandingArea_UnregisterFromExternalSystems_StowingVehicle",
            "CLandingArea::UnregisterFromExternalSystems",
            "Local Route Guard - Server Rerouted",
        ] {
            assert!(tag_is_reportable(good), "should send: {good}");
        }
        for bad in [
            "",
            "[STOWING ON UNREGISTER] LandingArea_X [745597122922]",
            "<CLandingArea::UnregisterFromExternalSystems>",
            "player {A27E3980-7BC8-42F5-A348-32E97E567C8B}",
            "name=\"SomePlayer\"",
            "-----",
        ] {
            assert!(!tag_is_reportable(bad), "must NOT send: {bad:?}");
        }
        assert!(!tag_is_reportable(&"x".repeat(201)));
    }

    #[test]
    fn heartbeat_due_respects_the_floor() {
        let now = Instant::now();
        // Never fired → always due.
        assert!(heartbeat_due(None, now));
        // Just fired → not due.
        assert!(!heartbeat_due(Some(now), now));
        // Fired within the floor → not due.
        let recent = now.checked_sub(HEARTBEAT_MIN_INTERVAL / 2).unwrap();
        assert!(!heartbeat_due(Some(recent), now));
        // Fired longer ago than the floor → due again.
        let old = now
            .checked_sub(HEARTBEAT_MIN_INTERVAL + Duration::from_secs(1))
            .unwrap();
        assert!(heartbeat_due(Some(old), now));
    }
}

/// How long a lane sleeps before its next attempt. With no recent failures
/// it's the configured `interval`; after `consecutive_failures` it backs off
/// exponentially (base, 2×, 4×, …) capped at [`MAX_BACKOFF`]. Without this a
/// failing drain retried every interval — the priority lane's 1s floor
/// hammered a down or erroring server. Pure so it can be unit-tested.
fn backoff_delay(interval: Duration, consecutive_failures: u32) -> Duration {
    if consecutive_failures == 0 {
        return interval;
    }
    // First failure retries at the base interval; each further failure
    // doubles. Clamp the shift so `1 << shift` can't overflow (2^20 already
    // dwarfs MAX_BACKOFF for any sane interval), and `checked_mul` guards the
    // Duration multiply itself.
    let shift = (consecutive_failures - 1).min(20);
    interval
        .checked_mul(1u32 << shift)
        .unwrap_or(MAX_BACKOFF)
        .min(MAX_BACKOFF)
}

#[cfg(test)]
mod backoff_tests {
    use super::*;

    #[test]
    fn backoff_is_base_when_healthy_then_doubles_and_caps() {
        let base = Duration::from_secs(1);
        assert_eq!(backoff_delay(base, 0), Duration::from_secs(1)); // healthy
        assert_eq!(backoff_delay(base, 1), Duration::from_secs(1)); // 1st failure = base
        assert_eq!(backoff_delay(base, 2), Duration::from_secs(2));
        assert_eq!(backoff_delay(base, 3), Duration::from_secs(4));
        assert_eq!(backoff_delay(base, 4), Duration::from_secs(8));
        // Caps at MAX_BACKOFF and never overflows for a runaway failure count.
        assert_eq!(backoff_delay(base, 100), MAX_BACKOFF);
        assert_eq!(backoff_delay(base, u32::MAX), MAX_BACKOFF);
        // A larger base interval is honored when healthy.
        assert_eq!(
            backoff_delay(Duration::from_secs(5), 0),
            Duration::from_secs(5)
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_lane(
    lane: Lane,
    interval: Duration,
    client: reqwest::Client,
    api_url: String,
    access_token: String,
    claimed_handle: String,
    storage: Arc<Storage>,
    sync_stats: Arc<parking_lot::Mutex<SyncStats>>,
    account_status: Arc<parking_lot::Mutex<AccountStatus>>,
    kick: Arc<SyncKick>,
    priority_types: Vec<String>,
    tuning: DrainTuning,
    sync_with_cloud: bool,
    app_handle: tauri::AppHandle,
    location_catalog: Arc<parking_lot::RwLock<Arc<LocationCatalog>>>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        // Consecutive drain failures, for the inter-cycle backoff. Reset to 0
        // on any successful drain; grows the sleep exponentially otherwise.
        let mut consecutive_failures: u32 = 0;
        // When the bulk lane last sent an idle heartbeat, for the rate-limit.
        let mut last_heartbeat: Option<Instant> = None;
        // When the bulk lane last pushed the opt-in unknown-tag report.
        let mut last_tag_report: Option<Instant> = None;
        // True while the previous drain came back with a FULL page, i.e.
        // there is a backlog behind it. Drives both the page size and the
        // inter-cycle delay — see DrainTuning. Starts false so a freshly
        // spawned lane makes one normal-sized probe drain before deciding
        // it is backlogged.
        let mut catching_up = false;
        // Whether Star Citizen is up. Only consulted while catching up, so
        // a lane with an empty queue never pays for a process scan.
        let mut game_probe = GameProbe::new();
        loop {
            // If a previous iteration tripped `auth_lost`, EXIT the
            // worker entirely. Previously this loop kept spinning,
            // skipping the drain on every tick — workers stayed alive
            // but did no work, and `sync_stats` kept its last-known-
            // good values, so the Settings health pill happily showed
            // green for hours after sync had silently died. Surfaced
            // 2026-05-28 in a tray "looks connected but isn't shipping"
            // outage. Now: log once, emit `sync-paused` so the UI can
            // show a banner, and break. The worker re-spawns on the
            // next `respawn()` call (pair_device, save_config,
            // set_sync_preset).
            if account_status.lock().auth_lost {
                tracing::warn!(
                    lane = lane.label(),
                    "sync worker exiting: auth_lost is set — waiting for re-pair"
                );
                if let Err(e) = app_handle.emit("sync-paused", "auth_lost") {
                    tracing::warn!(error = %e, "emit sync-paused failed");
                }
                // Re-emit the current on-disk config so the React side
                // catches up to whatever `clear_persisted_device_token`
                // already wrote (it blanks `access_token` and
                // `claimed_handle` but is called from contexts without
                // `AppHandle`, so it can't emit itself). Without this,
                // the Settings pane's "Paired as" card stays mounted
                // showing the stale handle until the user manually
                // clicks Unpair to force a refresh. Surfaced
                // 2026-05-29 on tray-v1.8.12 after a real-revoke test
                // — pill flipped to PAUSED correctly but the pair UI
                // didn't transition to the "enter code" state.
                match crate::config::load() {
                    Ok(cleared) => {
                        if let Err(e) = app_handle.emit("config-changed", &cleared) {
                            tracing::warn!(
                                error = %e,
                                "emit config-changed (auth_lost) failed"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "auth_lost: config load failed; UI may show stale paired state"
                        );
                    }
                }
                break;
            }
            // Auth gate passed — proceed with the regular drain/heartbeat cycle.
            {
                // Only probe the process table while backlogged: an idle
                // lane must not scan every tick, and the answer is
                // irrelevant unless it would change the page size.
                let game_running = if catching_up && tuning.catch_up_enabled {
                    game_probe.is_running()
                } else {
                    false
                };
                let page_size = tuning.page_size(catching_up, game_running);
                let types_ref: Vec<&str> = priority_types.iter().map(|s| s.as_str()).collect();
                match drain_lane(
                    lane,
                    &client,
                    &api_url,
                    &access_token,
                    &claimed_handle,
                    &storage,
                    &types_ref,
                    page_size,
                    tuning.max_batch_bytes,
                    &sync_stats,
                    &account_status,
                    &location_catalog,
                    &mut last_heartbeat,
                    &mut last_tag_report,
                )
                .await
                {
                    Ok(outcome) => {
                        // Reachable server (even an empty drain) clears the
                        // backoff so the lane returns to its normal cadence.
                        consecutive_failures = 0;
                        // A full page means more rows are waiting: loop again
                        // almost immediately instead of sleeping the interval.
                        // A short page means the queue is drained — fall back
                        // to the configured cadence.
                        catching_up = outcome.page_was_full;
                        if catching_up {
                            tracing::debug!(
                                lane = lane.label(),
                                page_size,
                                game_running,
                                "drain: page came back full — staying in catch-up"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(lane = lane.label(), error = %e, "sync drain failed");
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        // Leave catch-up on a failure: backoff_delay owns the
                        // cadence from here, and hot-looping a failing server
                        // at 250 ms is exactly what the backoff exists to stop.
                        catching_up = false;
                        let mut s = sync_stats.lock();
                        s.last_error = Some(format!("{}: {e}", lane.label()));
                    }
                }

                // Piggyback: bulk lane only. Pull preferences on the
                // same tick as the event drain — free roundtrip. The
                // priority lane ticks far more frequently (default 5s)
                // and never piggybacks.
                // `!catching_up`: a backlog drain ticks every 250 ms, and
                // piggybacking a preferences GET onto each of those would
                // turn a "free roundtrip" into thousands of requests. The
                // pull resumes on the first tick after the queue empties.
                if lane == Lane::Bulk && sync_with_cloud && !catching_up {
                    // Load the on-disk config and overlay the worker's
                    // captured api_url/access_token. We MUST start from
                    // disk, not from a synthetic Default::default()
                    // Config: piggyback returns Changed(next) which the
                    // arm below persists, so any field stripped here
                    // would be wiped on disk. See build_piggyback_input.
                    let piggyback_cfg = match crate::config::load() {
                        Ok(on_disk) => {
                            Some(build_piggyback_input(on_disk, &api_url, &access_token))
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "bulk tick: load on-disk config for piggyback failed; \
                                 skipping pull this tick"
                            );
                            None
                        }
                    };
                    if let Some(piggyback_cfg) = piggyback_cfg {
                        match piggyback_preferences_pull(&piggyback_cfg).await {
                            PiggybackOutcome::Changed(next) => {
                                let next_cfg = *next;
                                if let Err(e) = crate::config::save(&next_cfg) {
                                    tracing::warn!(error = %e, "persist piggyback-changed config failed");
                                } else {
                                    tracing::info!("bulk tick: preferences pulled and persisted");
                                    if let Err(e) = app_handle.emit("config-changed", &next_cfg) {
                                        tracing::warn!(error = %e, "bulk tick: emit config-changed failed");
                                    }
                                }
                            }
                            PiggybackOutcome::Revoked => {
                                // Server reports cloud sync is disabled
                                // for this uplink (either it was turned
                                // off elsewhere, or it was never enabled
                                // for this freshly-paired device). Flip
                                // the local flag and persist so the next
                                // respawn won't re-enable it; user sees
                                // the in-app notice and can flip it
                                // back on when ready.
                                match crate::config::load() {
                                    Ok(mut reverted) => {
                                        reverted.sync_with_cloud = false;
                                        if let Err(e) = crate::config::save(&reverted) {
                                            tracing::warn!(
                                                error = %e,
                                                "persist sync-revoked config failed"
                                            );
                                        } else {
                                            tracing::warn!(
                                                "bulk tick: server reports cloud sync disabled \
                                                 for this uplink; sync_with_cloud disabled locally"
                                            );
                                            if let Err(e) = app_handle.emit("sync-revoked", ()) {
                                                tracing::warn!(error = %e, "bulk tick: emit sync-revoked failed");
                                            }
                                            if let Err(e) =
                                                app_handle.emit("config-changed", &reverted)
                                            {
                                                tracing::warn!(error = %e, "bulk tick: emit config-changed (revoked) failed");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            "sync revoked but config reload failed"
                                        );
                                    }
                                }
                            }
                            PiggybackOutcome::Unchanged | PiggybackOutcome::Skipped => {}
                        }
                    }
                }
            }
            // Race the sleep against a manual kick. Whichever fires first
            // wins; the next iteration runs immediately. On failures the
            // sleep backs off exponentially so a down server isn't hammered,
            // but a manual "Sync now" still cuts through the backoff.
            // Re-read the (TTL-cached) probe: `catching_up` may have just
            // flipped true on a lane that skipped the probe above, and the
            // in-game pacing must apply from the very first catch-up tick.
            let game_running_now = if catching_up && tuning.catch_up_enabled {
                game_probe.is_running()
            } else {
                false
            };
            let delay = tuning.delay(
                catching_up,
                game_running_now,
                interval,
                consecutive_failures,
            );
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = kick.for_lane(lane).notified() => {}
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn drain_lane(
    lane: Lane,
    client: &reqwest::Client,
    api_url: &str,
    access_token: &str,
    claimed_handle: &str,
    storage: &Storage,
    priority_types: &[&str],
    page_size: usize,
    max_batch_bytes: usize,
    sync_stats: &parking_lot::Mutex<SyncStats>,
    account_status: &parking_lot::Mutex<AccountStatus>,
    location_catalog: &parking_lot::RwLock<Arc<LocationCatalog>>,
    last_heartbeat: &mut Option<Instant>,
    last_tag_report: &mut Option<Instant>,
) -> Result<DrainOutcome> {
    let pending = match lane {
        Lane::Priority => {
            // Fast lane: rows whose type IS IN the priority list.
            storage.read_unsent_filtered(priority_types, true, page_size)?
        }
        Lane::Bulk => {
            // Bulk lane: everything ELSE. When priority_types is
            // empty (fast lane disabled), this collapses to "all
            // unsent rows" — the legacy single-lane behaviour.
            storage.read_unsent_filtered(priority_types, false, page_size)?
        }
    };

    // A full page is the cheap "there is more behind this" signal the
    // caller uses to stay in catch-up. Captured BEFORE `pending` is
    // consumed by the chunker below.
    let page_was_full = pending.len() >= page_size;

    if pending.is_empty() {
        // Bulk lane owns the idle heartbeat, rate-limited to at most once per
        // HEARTBEAT_MIN_INTERVAL — a low bulk `interval_secs` (5s floor) would
        // otherwise hit /v1/auth/me on every empty drain. The priority lane
        // ticks every 5s and never heartbeats.
        // Opt-in parser-health tag report. Bulk lane only, rate-limited to
        // TAG_REPORT_MIN_INTERVAL, and re-reads the flag from disk each time
        // so revoking consent takes effect on the next tick rather than
        // needing a restart. A failure here is diagnostic-only and must
        // never disturb event sync, so it warns and moves on.
        let now_tags = Instant::now();
        if lane == Lane::Bulk && tag_report_due(*last_tag_report, now_tags) {
            let opted_in = crate::config::load()
                .map(|c| c.share_unknown_tags)
                .unwrap_or(false);
            if opted_in {
                *last_tag_report = Some(now_tags);
                match push_unknown_tags(api_url, access_token, storage).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(tags = n, "reported unknown shell tags"),
                    Err(e) => tracing::warn!(error = %e, "unknown-tag report failed"),
                }
            }
        }

        let now = Instant::now();
        if lane == Lane::Bulk && heartbeat_due(*last_heartbeat, now) {
            *last_heartbeat = Some(now);
            match fetch_me(api_url, access_token).await {
                Ok(Some(_)) => {
                    // Healthy. Nothing to do — keep `auth_lost` in
                    // whatever state it was. (We don't *clear*
                    // auth_lost here: that only happens via re-pair
                    // or the explicit refresh_account_status command.
                    // Auto-clearing on a successful heartbeat would
                    // race with a token that was just revoked and is
                    // about to be observed dead on the next /v1/ingest.)
                }
                Ok(None) => {
                    // 401/403 from /v1/auth/me — device token is dead.
                    // Same handling as the /v1/ingest 401 path below:
                    // clear the persisted token, flip auth_lost so the
                    // UI banner fires immediately (not 6 hours later
                    // when the queue happens to fill up), bail so the
                    // worker sleeps before its next iteration.
                    //
                    // Without this, the heartbeat could detect a dead
                    // token but the worker would keep the auth_lost
                    // flag at false until /v1/ingest itself returned
                    // 401 — observed in production at 2026-05-19
                    // 01:59:02Z. See StarStats CHANGELOG.
                    tracing::warn!(
                        lane = lane.label(),
                        "heartbeat: /v1/auth/me rejected device token — clearing and pausing sync"
                    );
                    if let Err(e) = clear_persisted_device_token() {
                        tracing::warn!(error = %e, "failed to clear device token after heartbeat auth loss");
                    }
                    {
                        let mut s = account_status.lock();
                        s.auth_lost = true;
                    }
                    let mut s = sync_stats.lock();
                    s.last_attempt_at = Some(now_rfc3339());
                    anyhow::bail!("auth lost: /v1/auth/me returned 401/403");
                }
                Err(e) => {
                    // Network or 5xx — not auth loss. Treat as a soft
                    // failure (the worker doesn't have anything to drain
                    // anyway).
                    tracing::debug!(error = %e, "heartbeat (GET /v1/auth/me) failed");
                }
            }
            let mut s = sync_stats.lock();
            s.last_attempt_at = Some(now_rfc3339());
        }
        return Ok(DrainOutcome {
            page_was_full: false,
        });
    }

    tracing::info!(
        lane = lane.label(),
        pending = pending.len(),
        page_size,
        "drain: starting"
    );

    {
        let mut s = sync_stats.lock();
        s.last_attempt_at = Some(now_rfc3339());
    }

    // Poison-pill isolation: a depth-first stack of sub-batches. We
    // start with the full pending set; on a 4xx (non-auth) we split
    // the failing batch in half and retry each half. When a
    // single-event batch still fails, we quarantine it so the rest of
    // the queue can move. Safety cap: at most `MAX_QUARANTINES_PER_DRAIN`
    // events are quarantined per drain pass to avoid catastrophic
    // mass-quarantine if the server is misconfigured (e.g.
    // schema-version skew rejecting every event).
    const MAX_QUARANTINES_PER_DRAIN: u32 = 10;

    // Snapshot the location catalogue once for the whole drain — a
    // cheap `Arc` clone, NOT a guard held across the awaiting send loop
    // below (parking_lot guards aren't `Send`). A catalogue hot-swap
    // mid-drain is picked up on the next drain; events in THIS pass all
    // classify against one consistent snapshot.
    let catalog = location_catalog.read().clone();

    // Snapshot the adopted rule-set version once for the whole drain so
    // every sub-batch (including poison-pill bisections) reports the same
    // provenance. A read failure degrades to `None` ("unknown rule-set")
    // rather than aborting the drain — provenance is best-effort metadata.
    let parser_version = storage
        .read_parser_def_manifest_version()
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "read_parser_def_manifest_version failed; batch parser_version=None");
            None
        });

    // Split the page into byte-bounded sub-batches BEFORE the first
    // send, so a large `page_size` never produces a body the server
    // rejects. Reversed because the loop below pops from the end: this
    // keeps sends in ascending-id order.
    let mut stack: Vec<Vec<UnsentEvent>> = chunk_by_bytes(pending, max_batch_bytes);
    stack.reverse();
    let mut total_sent = 0usize;
    let mut total_accepted = 0u32;
    let mut total_duplicate = 0u32;
    let mut total_rejected = 0u32;
    let mut total_quarantined = 0u32;

    while let Some(sub_batch) = stack.pop() {
        // Peek (do NOT consume) the next per-device batch ordinal. It is
        // committed only on a 2xx below, so a poison-bisected or retried
        // send reuses the same number rather than burning it into a false
        // server-side gap. A peek failure degrades to `None` — best-effort
        // metadata never aborts a send.
        let batch_sequence = storage
            .peek_next_batch_sequence()
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "peek_next_batch_sequence failed; batch_sequence=None");
                None
            });

        let response = try_send_batch(
            client,
            api_url,
            access_token,
            claimed_handle,
            &sub_batch,
            &catalog,
            parser_version,
            batch_sequence,
        )
        .await;

        match response {
            Ok(parsed) => {
                let ids: Vec<i64> = sub_batch.iter().map(|e| e.id).collect();
                storage.mark_sent(&ids)?;
                // Success (2xx) — consume the ordinal so the next batch
                // gets a fresh number. Best-effort: a commit failure just
                // means the next send may reuse this number (a benign
                // duplicate to the server), never a lost or corrupt
                // sequence.
                if let Some(seq) = batch_sequence {
                    if let Err(e) = storage.commit_batch_sequence(seq) {
                        tracing::warn!(error = %e, batch_sequence = seq, "commit_batch_sequence failed");
                    }
                }
                total_sent += ids.len();
                total_accepted += parsed.accepted;
                total_duplicate += parsed.duplicate;
                total_rejected += parsed.rejected;
            }
            Err(SendError::Auth { status, body }) => {
                // `device_sync_disabled` is NOT auth loss. A freshly-paired
                // device has `sync_enabled = false` server-side by default
                // (migration 0036), so /v1/ingest 403s with this error until
                // the user turns sync on — the device is still validly paired.
                // Clearing the token here (as for a revoked token) wiped
                // claimed_handle + access_token and UNPAIRED every fresh device
                // in a loop. Keep the pairing; just pause this drain (it backs
                // off) so the device stays linked and resumes once sync is
                // enabled. Mirrors the piggyback path's `Revoked` handling.
                if is_sync_disabled_rejection(&body) {
                    tracing::warn!(
                        lane = lane.label(),
                        %status,
                        "ingest paused: sync is disabled for this uplink — \
                         device stays paired, enable sync to resume"
                    );
                    anyhow::bail!("sync disabled for this uplink (device stays paired)");
                }
                // Genuine auth invalidation — token rejected (revoked device,
                // deleted account, signature invalid). Drop the stored token so
                // we don't keep re-trying with garbage, flip the UI flag, and
                // bail. Rows in this batch stay unsent and are re-picked
                // verbatim once the user re-pairs.
                tracing::warn!(
                    lane = lane.label(),
                    %status,
                    body = %body,
                    "ingest rejected device token — clearing and pausing sync"
                );
                if let Err(e) = clear_persisted_device_token() {
                    tracing::warn!(error = %e, "failed to clear device token after auth loss");
                }
                {
                    let mut s = account_status.lock();
                    s.auth_lost = true;
                }
                anyhow::bail!("auth lost: ingest returned {status}");
            }
            Err(SendError::BadBatch { status, body }) => {
                if sub_batch.len() == 1 {
                    if total_quarantined >= MAX_QUARANTINES_PER_DRAIN {
                        // Cap hit — bail without quarantining more.
                        // The remaining stack stays unsent; the next
                        // iteration will re-attempt the whole queue
                        // and re-bisect from scratch.
                        tracing::warn!(
                            lane = lane.label(),
                            %status,
                            quarantined_this_drain = total_quarantined,
                            remaining_subbatches = stack.len(),
                            "quarantine cap hit — bailing this drain"
                        );
                        anyhow::bail!(
                            "ingest 4xx loop: quarantine cap ({}) reached this drain",
                            MAX_QUARANTINES_PER_DRAIN
                        );
                    }
                    let id = sub_batch[0].id;
                    let idem = &sub_batch[0].idempotency_key;
                    tracing::warn!(
                        lane = lane.label(),
                        %status,
                        body = %body,
                        event_id = id,
                        idempotency_key = %idem,
                        raw_line_len = sub_batch[0].raw_line.len(),
                        "ingest rejected single event — quarantining"
                    );
                    storage.mark_quarantined(&[id])?;
                    total_quarantined += 1;
                } else {
                    // Split in half (LIFO: push right then left so we
                    // process left first, preserving roughly-ascending
                    // id order).
                    let mut left = sub_batch;
                    let mid = left.len() / 2;
                    let right = left.split_off(mid);
                    tracing::debug!(
                        lane = lane.label(),
                        %status,
                        left_len = left.len(),
                        right_len = right.len(),
                        "ingest 4xx — bisecting batch"
                    );
                    stack.push(right);
                    stack.push(left);
                }
            }
            Err(SendError::Transient(e)) => {
                // Network blip or 5xx — don't quarantine. Bail and let
                // the next iteration retry the whole queue. Any sub-
                // batches still on the stack are dropped on the floor
                // here; they remain `sent_at IS NULL` and get re-read
                // next iteration. `return Err(e)` rather than `bail!`
                // preserves the original error chain.
                return Err(e);
            }
        }
    }

    {
        let mut s = sync_stats.lock();
        s.last_success_at = Some(now_rfc3339());
        s.last_error = None;
        s.batches_sent += 1;
        s.events_accepted += total_accepted as u64;
        s.events_duplicate += total_duplicate as u64;
        s.events_rejected += total_rejected as u64;
        s.events_quarantined += total_quarantined as u64;
    }

    tracing::info!(
        lane = lane.label(),
        sent = total_sent,
        accepted = total_accepted,
        duplicate = total_duplicate,
        rejected = total_rejected,
        quarantined = total_quarantined,
        page_was_full,
        "drain: batch shipped"
    );

    Ok(DrainOutcome { page_was_full })
}

/// Outcome of one POST /v1/ingest attempt, classified so the caller
/// (the poison-pill loop) can decide whether to mark-sent, flip
/// auth_lost, bisect, or just retry the whole batch later.
enum SendError {
    /// 401/403 — device token rejected. The caller should clear the
    /// persisted token + flip auth_lost + bail.
    Auth { status: StatusCode, body: String },
    /// 4xx other than 401/403 — the SERVER is rejecting THIS batch
    /// content (schema-version skew, oversized payload, missing
    /// required field). The caller bisects to isolate the offender.
    BadBatch { status: StatusCode, body: String },
    /// Network error, 5xx, or response-parse failure. Treat as
    /// transient — don't quarantine, let the next iteration retry the
    /// whole queue.
    Transient(anyhow::Error),
}

#[allow(clippy::too_many_arguments)]
async fn try_send_batch(
    client: &reqwest::Client,
    api_url: &str,
    access_token: &str,
    claimed_handle: &str,
    events: &[UnsentEvent],
    catalog: &LocationCatalog,
    parser_version: Option<u32>,
    batch_sequence: Option<u64>,
) -> Result<IngestResponse, SendError> {
    let batch = build_batch(
        claimed_handle,
        events,
        catalog,
        parser_version,
        batch_sequence,
    );
    let url = format!("{}/v1/ingest", api_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .bearer_auth(access_token)
        .json(&batch)
        .send()
        .await
        .map_err(|e| SendError::Transient(anyhow::Error::from(e).context("POST /v1/ingest")))?;

    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        let body = resp.text().await.unwrap_or_default();
        return Err(SendError::Auth { status, body });
    }
    if status.is_client_error() {
        let body = resp.text().await.unwrap_or_default();
        return Err(SendError::BadBatch { status, body });
    }
    if !status.is_success() {
        // 5xx and anything else non-2xx — transient. Don't bisect.
        let body = resp.text().await.unwrap_or_default();
        return Err(SendError::Transient(anyhow::anyhow!(
            "ingest failed: {status} {body}"
        )));
    }
    resp.json::<IngestResponse>()
        .await
        .map_err(|e| SendError::Transient(anyhow::Error::from(e).context("parse ingest response")))
}

/// Wipe the persisted device token + claimed_handle from the on-disk
/// config. `enabled` is left as-is — re-pairing will re-fill the
/// fields and resume the worker. Idempotent: safe to call when the
/// token is already absent.
///
/// `pub(crate)` so the hangar fetcher (`hangar.rs`) and any future
/// auth-aware caller can reuse the same clear-token dance instead of
/// duplicating it. The sync worker, hangar push, and (future)
/// parser-submission flow all share one identity, so one of them
/// detecting auth loss should invalidate it for all.
pub(crate) fn clear_persisted_device_token() -> Result<()> {
    let mut cfg = config::load().context("load config to clear token")?;
    cfg.remote_sync.access_token = None;
    cfg.remote_sync.claimed_handle = None;
    config::save(&cfg).context("save config after clearing token")?;
    // M-T6: the token now lives in the OS keychain, and `save` with a None
    // token deliberately leaves the keychain untouched — so we MUST clear it
    // here explicitly. Otherwise the next `config::load` re-hydrates the
    // just-revoked token from the keychain and auth-loss never sticks.
    // Best-effort: a keychain hiccup must not fail the auth-clear path.
    if let Err(e) = crate::secret::SecretStore::new(crate::secret::ACCOUNT_DEVICE_TOKEN)
        .and_then(|store| store.clear())
    {
        tracing::warn!(error = %e, "failed to clear device token from keychain");
    }
    Ok(())
}

/// One-shot HTTP call to `GET /v1/auth/me`. Used on startup and after
/// pairing to populate the account status surface (email-verified
/// banner, future: avatar / display name).
///
/// Returns `Ok(None)` on 401/403 — auth was already lost; caller
/// should reflect that in `AccountStatus`. Returns `Err` on
/// network/5xx errors so the caller can decide whether to retry.
/// Push unclassified shell-tag METADATA so the server can correlate a
/// parser break with the tag that caused it.
///
/// Sends `shell_tag` + sighting window + count. Never a log line body: the
/// storage projection does not even read `raw_examples_json`, and every tag
/// is re-checked here before send. Caller gates on
/// `Config::share_unknown_tags`; this function does not decide policy.
pub async fn push_unknown_tags(
    api_url: &str,
    access_token: &str,
    storage: &Storage,
) -> Result<usize> {
    let rows = storage
        .unknown_tag_metadata()
        .context("read unknown tag metadata")?;
    let payload: Vec<serde_json::Value> = rows
        .into_iter()
        .filter(|r| tag_is_reportable(&r.shell_tag))
        .map(|r| {
            serde_json::json!({
                "shell_tag": r.shell_tag,
                "first_seen": r.first_seen,
                "last_seen": r.last_seen,
                "occurrences": r.occurrences,
                "game_build": r.game_build,
            })
        })
        .collect();
    if payload.is_empty() {
        return Ok(0);
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build http client")?;
    let url = format!("{}/v1/unknown-tags", api_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .bearer_auth(access_token)
        .json(&serde_json::json!({ "tags": payload }))
        .send()
        .await
        .context("POST /v1/unknown-tags")?;

    if !resp.status().is_success() {
        anyhow::bail!("unknown-tags report rejected: {}", resp.status());
    }
    Ok(payload.len())
}

/// Per-type event counts as the SERVER sees them. Mirrors the server's
/// `query::SummaryResponse`; duplicated rather than depending on the server
/// crate, same as [`MeResponse`].
#[derive(Debug, Clone, Deserialize)]
pub struct SummaryCounts {
    pub total: u64,
    pub by_type: Vec<SummaryTypeCount>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SummaryTypeCount {
    pub event_type: String,
    pub count: u64,
}

/// Read the server's per-type event counts for the authenticated user.
///
/// Deliberately `GET /v1/me/summary` rather than a purpose-built endpoint:
/// that handler is already "hard-cut to the stat_event_counts rollup", so it
/// answers from a handful of indexed rows keyed by (claimed_handle,
/// event_type) instead of scanning the events table. Drift detection
/// therefore costs the server one small read it already serves for the web
/// dashboard — no new route, no new query shape, no migration.
///
/// Everything else (the comparison, deciding what is missing) happens on the
/// client, which already holds its own counts.
///
/// Returns `Ok(None)` on 401/403 so the caller can report auth loss rather
/// than a confusing parse error.
pub async fn fetch_summary(api_url: &str, access_token: &str) -> Result<Option<SummaryCounts>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("build http client")?;

    let url = format!("{}/v1/me/summary", api_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .context("GET /v1/me/summary")?;

    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        tracing::warn!(%status, "GET /v1/me/summary rejected device token");
        return Ok(None);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("GET /v1/me/summary failed: {status} {body}");
    }
    let counts: SummaryCounts = resp.json().await.context("parse SummaryResponse")?;
    Ok(Some(counts))
}

pub async fn fetch_me(api_url: &str, access_token: &str) -> Result<Option<MeResponse>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("build http client")?;

    let url = format!("{}/v1/auth/me", api_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .context("GET /v1/auth/me")?;

    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        tracing::warn!(%status, "GET /v1/auth/me rejected token");
        return Ok(None);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("GET /v1/auth/me failed: {status} {body}");
    }
    let me: MeResponse = resp.json().await.context("parse MeResponse")?;
    Ok(Some(me))
}

/// Stable namespace for batch content-hash v5 UUIDs (an arbitrary fixed
/// constant — only its stability matters).
const BATCH_CONTENT_HASH_NS: uuid::Uuid =
    uuid::Uuid::from_u128(0x9f2b_1c7a_4e33_4d21_8a6f_1b0c_5e7d_9a42);

/// Content-address a batch by its event SET: a UUIDv5 over the batch's
/// idempotency keys, sorted. Sorting makes it order-independent (a
/// re-drain that reorders the same events hashes identically); v5 is a
/// fixed SHA-1-based algorithm, so the value is stable across machines and
/// toolchains. Gives the server a batch-level dedup / replay + integrity
/// signal beyond per-event idempotency. An empty batch hashes the empty
/// string — still well-defined.
fn compute_content_hash(events: &[UnsentEvent]) -> String {
    let mut keys: Vec<&str> = events.iter().map(|e| e.idempotency_key.as_str()).collect();
    keys.sort_unstable();
    uuid::Uuid::new_v5(&BATCH_CONTENT_HASH_NS, keys.join("\n").as_bytes()).to_string()
}

/// The byte span this batch covers within its log source — but ONLY when
/// every event shares one `log_source` (the common live-tail case). A
/// drain batches by event TYPE, not by source, so a batch can mix the live
/// tail and the launcher log, whose `source_offset`s reset per file and
/// aren't comparable; a mixed-source batch (and an empty one) ships `None`
/// rather than a meaningless range.
fn compute_source_range(events: &[UnsentEvent]) -> Option<SourceRange> {
    let first = events.first()?;
    let source = parse_source(&first.log_source);
    if events.iter().any(|e| parse_source(&e.log_source) != source) {
        return None;
    }
    let (mut start_offset, mut end_offset) = (first.source_offset, first.source_offset);
    for e in events {
        start_offset = start_offset.min(e.source_offset);
        end_offset = end_offset.max(e.source_offset);
    }
    Some(SourceRange {
        source,
        start_offset,
        end_offset,
    })
}

fn build_batch(
    claimed_handle: &str,
    events: &[UnsentEvent],
    catalog: &LocationCatalog,
    parser_version: Option<u32>,
    batch_sequence: Option<u64>,
) -> IngestBatch {
    let envelopes: Vec<EventEnvelope> = events
        .iter()
        .map(|e| {
            // The locally-stored payload SHOULD always parse — we
            // wrote it ourselves on the parser side. If it doesn't
            // parse here, something has gone wrong (schema drift in
            // GameEvent, db corruption); log with idempotency key so
            // it's traceable, and ship `event: None` — the server
            // accepts the envelope and stores `event_type=unknown`,
            // which is at least visible in the unknown-events query
            // rather than silently lost.
            let event: Option<starstats_core::events::GameEvent> =
                match serde_json::from_str(&e.payload_json) {
                    Ok(ev) => Some(ev),
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            idempotency_key = %e.idempotency_key,
                            log_source = %e.log_source,
                            source_offset = e.source_offset,
                            "stored event payload failed to deserialize; shipping as null"
                        );
                        None
                    }
                };
            // Stamp the fuzzy-resolved location once, here on the tray,
            // so the tray's recent-events view and the web event views
            // render IDENTICAL resolution (the classifier never re-runs
            // server-side). Only events that carry a location field
            // (`location_raw()` is `Some`) get a `resolved_location`;
            // `classify` itself guarantees a `display_name`/`tier` even
            // on a catalog miss, and surfaces a `slug` only on a
            // confident catalog/fuzzy hit. Placeless events stay `None`.
            let resolved_location = event
                .as_ref()
                .and_then(|ev| ev.location_raw())
                .map(|raw| ResolvedLocation::from(classify(raw, catalog)));

            EventEnvelope {
                idempotency_key: e.idempotency_key.clone(),
                raw_line: e.raw_line.clone(),
                event,
                source: parse_source(&e.log_source),
                source_offset: e.source_offset,
                // Per Phase 1.A: metadata stamping happens in a later
                // task; envelopes shipped today carry None and the
                // server back-fills observed metadata server-side.
                metadata: None,
                resolved_location,
            }
        })
        .collect();

    IngestBatch {
        schema_version: IngestBatch::CURRENT_SCHEMA_VERSION,
        batch_id: uuid::Uuid::now_v7().to_string(),
        game_build: None,
        // Stamp the collector release that produced this batch so the
        // server can attribute ingested events to a tray version (parser
        // -regression triage, future compatibility gating). Compile-time
        // constant — no runtime cost, always present on modern clients.
        collector_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        // Which remote rule-set the collector had adopted at drain time
        // (read from the persisted manifest by the caller). `None` until
        // the first manifest fetch lands.
        parser_version,
        // Per-device ordinal of this upload (peeked from the persisted
        // counter by the caller, committed only on a 2xx). Lets the
        // server spot gaps / out-of-order uploads from a device. `None`
        // when the peek failed (best-effort metadata never aborts a send).
        batch_sequence,
        // Content-address the event set (UUIDv5 over sorted idempotency
        // keys) so the server gets a batch-level dedup / integrity signal.
        content_hash: Some(compute_content_hash(events)),
        // Byte coverage span — populated only for single-source batches.
        source_range: compute_source_range(events),
        claimed_handle: claimed_handle.to_string(),
        events: envelopes,
    }
}

fn parse_source(s: &str) -> LogSource {
    match s {
        "live" => LogSource::Live,
        "ptu" => LogSource::Ptu,
        "eptu" => LogSource::Eptu,
        "hotfix" => LogSource::Hotfix,
        "tech" => LogSource::Tech,
        _ => LogSource::Other,
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Debounce gate for the window-show trigger. Returns `true` (and
/// updates `last`) if at least 5 seconds have passed since the
/// previous pull, or if this is the first call. Otherwise leaves
/// `last` unchanged and returns `false`.
///
/// `pub` so `main.rs` can call it from the `on_window_event` closure.
pub fn should_pull_on_focus(last: &mut Option<std::time::Instant>) -> bool {
    let now = std::time::Instant::now();
    match last {
        Some(t) if now.duration_since(*t) < std::time::Duration::from_secs(5) => false,
        _ => {
            *last = Some(now);
            true
        }
    }
}

/// Build the input Config for [`piggyback_preferences_pull`] from the
/// on-disk state plus the worker's captured api_url + access_token.
///
/// **Critical invariant:** this function must preserve every field of
/// `on_disk` that [`crate::config_sync::apply_remote_prefs`] doesn't
/// modify — most importantly `claimed_handle` and `gamelog_path`. An
/// earlier iteration constructed a synthetic `Config` from
/// `Default::default()` and forwarded only `api_url + access_token` from
/// captured locals; the resulting `PiggybackOutcome::Changed(next)` was
/// then persisted, wiping `claimed_handle` on disk. Symptom: tray
/// settings page reported the device as "off" and the queue stalled
/// shortly after pairing. The fix is to start from the on-disk Config
/// and only overlay the captured creds — see the bulk-tick caller in
/// `spawn_lane`.
///
/// `api_url` and `access_token` win over whatever's currently on disk:
/// the worker captures them at spawn time and respawn fires on every
/// config save, so captured values are the canonical "live" values for
/// this worker's lifetime.
pub(crate) fn build_piggyback_input(
    mut on_disk: crate::config::Config,
    api_url: &str,
    access_token: &str,
) -> crate::config::Config {
    on_disk.sync_with_cloud = true;
    on_disk.remote_sync.api_url = Some(api_url.to_string());
    on_disk.remote_sync.access_token = Some(access_token.to_string());
    on_disk
}

/// Outcome returned by [`piggyback_preferences_pull`]. Used by the
/// bulk lane and by the app-launch / window-show triggers (C7) to
/// decide whether to persist + emit `config-changed`.
#[derive(Debug)]
pub enum PiggybackOutcome {
    /// `sync_with_cloud` is false, or required config fields are absent.
    /// No network call was made.
    Skipped,
    /// Network call succeeded; remote preferences matched local config.
    Unchanged,
    /// Network call succeeded; remote preferences differed. The boxed
    /// value is the updated Config ready to persist. `Box<Config>`
    /// avoids the `large_enum_variant` lint (Config is ~400 bytes).
    Changed(Box<crate::config::Config>),
    /// Server returned 403 — the device's sync permission was revoked.
    /// Caller should set `sync_with_cloud = false`, persist, and
    /// surface a notification to the user.
    Revoked,
}

/// Pull preferences from the server and merge into a snapshot of
/// `current`. Returns [`PiggybackOutcome::Changed`] when remote
/// preferences differed from local; the caller is responsible for
/// persisting the new config and emitting `config-changed`.
///
/// Designed to be called once per bulk-lane tick so the preferences
/// read piggbacks on the same interval as the event drain. Also
/// re-used by the app-launch and window-show triggers (C7).
pub async fn piggyback_preferences_pull(current: &crate::config::Config) -> PiggybackOutcome {
    if !current.sync_with_cloud {
        return PiggybackOutcome::Skipped;
    }
    let api_url = match &current.remote_sync.api_url {
        Some(u) if !u.is_empty() => u.clone(),
        _ => return PiggybackOutcome::Skipped,
    };
    let token = match &current.remote_sync.access_token {
        Some(t) if !t.is_empty() => t.clone(),
        _ => return PiggybackOutcome::Skipped,
    };

    let client = reqwest::Client::new();
    match crate::preferences_client::get_preferences(&client, &api_url, &token).await {
        Ok(prefs) => {
            let mut next = current.clone();
            if crate::config_sync::apply_remote_prefs(&mut next, &prefs) {
                PiggybackOutcome::Changed(Box::new(next))
            } else {
                PiggybackOutcome::Unchanged
            }
        }
        Err(crate::preferences_client::PreferencesClientError::SyncDisabled) => {
            PiggybackOutcome::Revoked
        }
        Err(e) => {
            tracing::warn!(error = ?e, "preferences pull failed during bulk tick");
            PiggybackOutcome::Unchanged
        }
    }
}

#[cfg(test)]
mod launch_focus_trigger_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn debounce_blocks_pulls_under_5_seconds() {
        let mut last = Some(Instant::now() - Duration::from_secs(2));
        let original = last;
        assert!(!should_pull_on_focus(&mut last));
        // last should be unchanged when blocked
        assert_eq!(last, original);
    }

    #[test]
    fn debounce_allows_pulls_after_5_seconds() {
        let mut last = Some(Instant::now() - Duration::from_secs(6));
        assert!(should_pull_on_focus(&mut last));
        // last should be updated to ~now
        let elapsed = last.unwrap().elapsed();
        assert!(elapsed < Duration::from_secs(1));
    }

    #[test]
    fn debounce_allows_first_pull() {
        let mut last: Option<Instant> = None;
        assert!(should_pull_on_focus(&mut last));
        assert!(last.is_some());
    }
}

#[cfg(test)]
mod cloud_sync_piggyback_tests {
    use super::*;
    use crate::config::{Config, Theme};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_test_config(server_uri: String, sync_with_cloud: bool) -> Config {
        use crate::config::RemoteSyncConfig;
        Config {
            sync_with_cloud,
            remote_sync: RemoteSyncConfig {
                api_url: Some(server_uri),
                access_token: Some("tok".into()),
                claimed_handle: Some("U".into()),
                ..RemoteSyncConfig::default()
            },
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn bulk_tick_pulls_preferences_when_sync_enabled() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me/preferences"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "theme": "nyx"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut cfg = make_test_config(server.uri(), true);
        cfg.theme = Theme::Stanton;

        let outcome = piggyback_preferences_pull(&cfg).await;
        match outcome {
            PiggybackOutcome::Changed(c) => {
                let unboxed: Config = *c;
                assert_eq!(unboxed.theme, Theme::Nyx);
                // Regression lock: the Changed outcome MUST preserve
                // every input field that apply_remote_prefs doesn't
                // touch. Stripping claimed_handle here is what caused
                // tray sync to die shortly after pairing (settings
                // page showed device "off", queue stalled).
                assert_eq!(
                    unboxed.remote_sync.claimed_handle.as_deref(),
                    Some("U"),
                    "claimed_handle must survive a Changed outcome"
                );
                assert_eq!(
                    unboxed.remote_sync.access_token.as_deref(),
                    Some("tok"),
                    "access_token must survive a Changed outcome"
                );
            }
            other => panic!("expected Changed, got {other:?}"),
        }
    }

    #[test]
    fn build_piggyback_input_preserves_claimed_handle() {
        use crate::config::{Config, RemoteSyncConfig};
        let on_disk = Config {
            remote_sync: RemoteSyncConfig {
                claimed_handle: Some("DaisyHandle".into()),
                ..RemoteSyncConfig::default()
            },
            ..Config::default()
        };
        let prepared = build_piggyback_input(on_disk, "https://api.example/", "tok");
        assert_eq!(
            prepared.remote_sync.claimed_handle.as_deref(),
            Some("DaisyHandle"),
            "claimed_handle must survive the input-build step"
        );
        assert_eq!(
            prepared.remote_sync.api_url.as_deref(),
            Some("https://api.example/")
        );
        assert_eq!(prepared.remote_sync.access_token.as_deref(), Some("tok"));
        assert!(
            prepared.sync_with_cloud,
            "sync_with_cloud must be true at the input-build step (we only piggyback when it was true at spawn)"
        );
    }

    #[test]
    fn build_piggyback_input_preserves_gamelog_path() {
        use crate::config::Config;
        use std::path::{Path, PathBuf};
        let on_disk = Config {
            gamelog_path: Some(PathBuf::from(r"C:\StarCitizen\LIVE\Game.log")),
            ..Config::default()
        };
        let prepared = build_piggyback_input(on_disk, "https://api.example/", "tok");
        assert_eq!(
            prepared.gamelog_path.as_deref(),
            Some(Path::new(r"C:\StarCitizen\LIVE\Game.log")),
            "gamelog_path must survive the input-build step"
        );
    }

    #[test]
    fn build_piggyback_input_forwards_captured_creds_over_on_disk_values() {
        // The worker captures api_url/access_token at spawn time and
        // respawn fires on every config save — so captured values are
        // the canonical "live" values for this worker's lifetime. If
        // on-disk has different values (race window between save and
        // respawn), the captured values win.
        use crate::config::{Config, RemoteSyncConfig};
        let on_disk = Config {
            remote_sync: RemoteSyncConfig {
                api_url: Some("https://STALE.example/".into()),
                access_token: Some("STALE-tok".into()),
                claimed_handle: Some("Handle".into()),
                ..RemoteSyncConfig::default()
            },
            ..Config::default()
        };
        let prepared = build_piggyback_input(on_disk, "https://live.example/", "live-tok");
        assert_eq!(
            prepared.remote_sync.api_url.as_deref(),
            Some("https://live.example/")
        );
        assert_eq!(
            prepared.remote_sync.access_token.as_deref(),
            Some("live-tok")
        );
        // …but on-disk-only fields still flow through.
        assert_eq!(
            prepared.remote_sync.claimed_handle.as_deref(),
            Some("Handle")
        );
    }

    #[tokio::test]
    async fn bulk_tick_skips_when_sync_disabled() {
        // No mocks — any call fails the test.
        let cfg = make_test_config("http://unused.invalid".into(), false);
        let outcome = piggyback_preferences_pull(&cfg).await;
        assert!(matches!(outcome, PiggybackOutcome::Skipped));
    }

    #[tokio::test]
    async fn bulk_tick_returns_revoked_on_403() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me/preferences"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "error": "device_sync_disabled"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let cfg = make_test_config(server.uri(), true);
        let outcome = piggyback_preferences_pull(&cfg).await;
        assert!(matches!(outcome, PiggybackOutcome::Revoked));
    }

    #[tokio::test]
    async fn bulk_tick_returns_unchanged_when_remote_matches() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me/preferences"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "theme": "stanton"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut cfg = make_test_config(server.uri(), true);
        cfg.theme = Theme::Stanton;

        let outcome = piggyback_preferences_pull(&cfg).await;
        assert!(matches!(outcome, PiggybackOutcome::Unchanged));
    }

    #[test]
    fn content_hash_is_order_independent_and_source_range_is_single_source_only() {
        let ev = |id: i64, key: &str, src: &str, off: u64| UnsentEvent {
            id,
            idempotency_key: key.into(),
            payload_json: "{}".into(),
            raw_line: "<...>".into(),
            log_source: src.into(),
            source_offset: off,
        };

        // content_hash: order-independent (keys are sorted) + deterministic.
        let a = vec![ev(1, "k1", "live", 0), ev(2, "k2", "live", 1)];
        let b = vec![ev(2, "k2", "live", 1), ev(1, "k1", "live", 0)];
        assert_eq!(
            compute_content_hash(&a),
            compute_content_hash(&b),
            "reordering the same event set must not change the hash"
        );
        // A different event set hashes differently.
        let c = vec![ev(1, "k1", "live", 0), ev(3, "k3", "live", 2)];
        assert_ne!(compute_content_hash(&a), compute_content_hash(&c));

        // source_range: single-source → min..max; mixed → None; empty → None.
        assert_eq!(
            compute_source_range(&a),
            Some(SourceRange {
                source: LogSource::Live,
                start_offset: 0,
                end_offset: 1,
            })
        );
        let mixed = vec![ev(1, "k1", "live", 0), ev(2, "k2", "ptu", 9)];
        assert_eq!(
            compute_source_range(&mixed),
            None,
            "a mixed-source batch has no meaningful single range"
        );
        assert_eq!(compute_source_range(&[]), None);
    }

    #[test]
    fn build_batch_stamps_resolved_location_only_for_located_events() {
        use starstats_core::events::{GameEvent, JoinPu, PlanetTerrainLoad};

        // Empty catalogue: a located event still resolves (the
        // classifier always yields a display_name + tier) but to a
        // Fallback with NO slug — exercising the "stamp present, link
        // absent" wire shape without needing a populated catalogue.
        let catalog = LocationCatalog::from_entries(vec![]);

        let located = GameEvent::PlanetTerrainLoad(PlanetTerrainLoad {
            timestamp: "2026-06-03T00:00:00.000Z".into(),
            planet: "Crusader".into(),
        });
        let placeless = GameEvent::JoinPu(JoinPu {
            timestamp: "2026-06-03T00:00:00.000Z".into(),
            address: "1.2.3.4".into(),
            port: 64300,
            shard: "pub_euw1b".into(),
            location_id: "1".into(),
        });

        let events = vec![
            UnsentEvent {
                id: 1,
                idempotency_key: "evt-located".into(),
                payload_json: serde_json::to_string(&located).unwrap(),
                raw_line: "<...>".into(),
                log_source: "live".into(),
                source_offset: 0,
            },
            UnsentEvent {
                id: 2,
                idempotency_key: "evt-placeless".into(),
                payload_json: serde_json::to_string(&placeless).unwrap(),
                raw_line: "<...>".into(),
                log_source: "live".into(),
                source_offset: 1,
            },
        ];

        let batch = build_batch("alice", &events, &catalog, Some(7), Some(4));

        // Provenance stamped: the collector version is the compile-time
        // crate version and the parser version is whatever the caller
        // read from the persisted manifest.
        assert_eq!(
            batch.collector_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(batch.parser_version, Some(7));
        assert_eq!(batch.batch_sequence, Some(4));
        // Both events are "live" → single-source batch gets a byte span.
        assert_eq!(
            batch.source_range,
            Some(SourceRange {
                source: LogSource::Live,
                start_offset: 0,
                end_offset: 1,
            })
        );
        assert!(
            batch.content_hash.is_some(),
            "every modern batch is content-addressed"
        );

        let resolved = batch.events[0]
            .resolved_location
            .as_ref()
            .expect("a located event must carry a resolved_location");
        assert!(
            resolved.display_name.to_lowercase().contains("crusader"),
            "display_name should derive from the raw location, got {:?}",
            resolved.display_name
        );
        assert!(
            resolved.slug.is_none(),
            "empty catalogue → no catalog/fuzzy hit → no slug to link"
        );

        assert!(
            batch.events[1].resolved_location.is_none(),
            "a placeless event (JoinPu carries no location) must stay None"
        );
    }
}
