//! Tauri command surface — every function the JS frontend can call.
//!
//! Convention: errors stringify on the way out so the frontend gets a
//! human-readable message via the Tauri Promise rejection path.

use crate::backfill::BackfillStats;
use crate::config::{self, Config};
use crate::crashes::CrashStats;
use crate::discovery::{self, DiscoveredLog};
use crate::gamelog::TailStats;
use crate::hangar::HangarStats;
use crate::launcher::LauncherStats;
use crate::org_connector;
use crate::secret::{SecretStore, ACCOUNT_ORG_BEARER, ACCOUNT_RSI_SESSION_COOKIE};
use crate::state::{AccountStatus, AppState};
use crate::sync::{self, SyncStats};
use serde::{Deserialize, Serialize};
use starstats_core::templates::{
    build_loadout_categories, build_loadout_items, detect_bursts_with_time_gap,
};
use starstats_core::{
    apply_remote_rules, classify, pair_transactions, structural_parse, BurstSummary, GameEvent,
    LogLine, Transaction,
};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct StatusResponse {
    pub tail: TailStats,
    pub sync: SyncStats,
    /// Hangar refresh worker's last-seen state (last attempt, last
    /// success, last error, ships pushed, last skip reason). Surfaced
    /// alongside `tail` and `sync` so the existing webview status-poll
    /// loop covers it without a dedicated command.
    pub hangar: HangarStats,
    pub event_counts: Vec<EventCount>,
    pub total_events: u64,
    pub discovered_logs: Vec<DiscoveredLog>,
    /// Account-lifecycle signals — `auth_lost` (device token rejected
    /// by the API) and `email_verified` (mirror of `GET /v1/auth/me`).
    /// Driven by the sync worker and the startup / post-pair refresh.
    pub account: AccountStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventCount {
    pub event_type: String,
    pub count: u64,
}

/// Coverage report for the parser — what's recognised, what's
/// structurally-known but unclassified, what's totally skipped, and
/// a list of the top unknowns the user could potentially write rules
/// for.
#[derive(Debug, Clone, Serialize)]
pub struct ParseCoverageResponse {
    pub recognised: u64,
    pub structural_only: u64,
    pub skipped: u64,
    /// Lines whose event_name was on the noise list — recognised as
    /// engine-internal chatter and dropped on purpose. Counted so the
    /// user sees "we filtered N noise lines" rather than wondering
    /// where they went.
    pub noise: u64,
    pub unknowns: Vec<UnknownSample>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnknownSample {
    pub log_source: String,
    pub event_name: String,
    pub occurrences: u64,
    pub first_seen: String,
    pub last_seen: String,
    pub sample_line: String,
    pub sample_body: String,
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_status(state: State<'_, AppState>) -> Result<StatusResponse, String> {
    let tail = state.tail_stats.lock().clone();
    let sync = state.sync_stats.lock().clone();
    let hangar = state.hangar_stats.lock().clone();
    let account = state.account_status.lock().clone();
    let counts = state
        .storage
        .event_counts()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(event_type, count)| EventCount { event_type, count })
        .collect();
    let total = state.storage.total_events().map_err(|e| e.to_string())?;
    let discovered = discovery::discover();
    Ok(StatusResponse {
        tail,
        sync,
        hangar,
        event_counts: counts,
        total_events: total,
        discovered_logs: discovered,
        account,
    })
}

/// Re-fetch `GET /v1/auth/me` and update the in-memory account
/// snapshot. Called from the React side after a successful pair, and
/// once on startup. Returns the new `AccountStatus` so the caller can
/// reflect it immediately without a follow-up `get_status` round-trip.
///
/// On token absence (no pair yet) returns the current snapshot
/// unchanged. On 401/403 from the API, marks `auth_lost`. Network
/// errors are non-fatal — the snapshot keeps its previous value and
/// we surface the error string for the UI to optionally show.
#[tauri::command(rename_all = "snake_case")]
pub async fn refresh_account_info(state: State<'_, AppState>) -> Result<AccountStatus, String> {
    let cfg = config::load().map_err(|e| e.to_string())?;
    let (api_url, token) = match (
        cfg.remote_sync.api_url.as_deref(),
        cfg.remote_sync.access_token.as_deref(),
    ) {
        (Some(u), Some(t)) => (u.to_string(), t.to_string()),
        _ => return Ok(state.account_status.lock().clone()),
    };

    match sync::fetch_me(&api_url, &token).await {
        Ok(Some(me)) => {
            let mut s = state.account_status.lock();
            s.auth_lost = false;
            s.email_verified = Some(me.email_verified);
            Ok(s.clone())
        }
        Ok(None) => {
            // Server said the token is no longer valid. Treat the
            // same as the sync worker's auth-loss path so the UI
            // converges on a single state.
            let mut s = state.account_status.lock();
            s.auth_lost = true;
            Ok(s.clone())
        }
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_config() -> Result<Config, String> {
    let mut cfg = config::load().map_err(|e| e.to_string())?;
    // Resolve web_origin server-side before the TS sees it. When the
    // user hasn't explicitly configured it, the derived value
    // (`api.<host>` → `<host>`) is what the "Open on web" affordance
    // should use. The on-disk config still stores None — we're only
    // hydrating the returned shape so the TS has a single value to
    // read instead of duplicating the resolution logic.
    if cfg.web_origin.is_none() {
        cfg.web_origin = cfg.effective_web_origin();
    }
    Ok(cfg)
}

/// Outcome of a Rust-side updater check. Mirrors the JS `UpdateInfo`
/// type but with no opaque Update handle — the install path re-checks
/// internally because `tauri_plugin_updater::Update` isn't
/// Serializable and can't ride the IPC bridge.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckOutcome {
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

fn build_channel_updater(
    app: &tauri::AppHandle,
    channel: crate::config::ReleaseChannel,
) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    let url = channel
        .manifest_url()
        .parse::<tauri::Url>()
        .map_err(|e| format!("manifest URL did not parse: {e}"))?;
    let builder = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| format!("set endpoints: {e}"))?;
    builder.build().map_err(|e| format!("build updater: {e}"))
}

/// Check the given channel's manifest for a newer release.
///
/// We can't return the underlying `Update` handle to JS — its type
/// from `tauri-plugin-updater` isn't Serializable. Instead we return
/// just the metadata; the install command does its own check (the
/// race window is fine for our scale, and a new release between
/// check and install would simply install the newer one).
#[tauri::command(rename_all = "snake_case")]
pub async fn check_for_update_for_channel(
    channel: crate::config::ReleaseChannel,
    app: tauri::AppHandle,
) -> Result<UpdateCheckOutcome, String> {
    let updater = build_channel_updater(&app, channel)?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(u) => Ok(UpdateCheckOutcome {
            available: true,
            version: Some(u.version.clone()),
            notes: u.body.clone(),
            date: u.date.map(|d| d.to_string()),
        }),
        None => Ok(UpdateCheckOutcome {
            available: false,
            version: None,
            notes: None,
            date: None,
        }),
    }
}

/// Download + install the latest release on the given channel,
/// emitting `update-progress` events on the way through. The
/// frontend listens for these to drive its progress bar; on success
/// the process plugin's `relaunch()` swaps in the new binary, so
/// this command does not return.
///
/// If the manifest reports nothing newer (e.g. the user already
/// installed it via another path between check and install), this
/// returns `Ok(false)` so the UI can flip back to "up to date".
#[tauri::command(rename_all = "snake_case")]
pub async fn install_update_for_channel(
    channel: crate::config::ReleaseChannel,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    use tauri::Emitter;

    // Persist the user's channel choice BEFORE downloading. Installing
    // from <channel> is an opt-in commitment — without this, a user
    // can change the Settings dropdown to alpha, hit "Install Update"
    // without first hitting the form's Save button, and end up with an
    // alpha binary while config.toml still says "live". On restart the
    // new binary loads `release_channel = live` from disk and the
    // dropdown reverts, surprising the user. `check_for_update` stays
    // read-only since browsing what's on another channel shouldn't
    // commit; the commit only happens when the user actually installs.
    {
        let mut cfg = config::load().unwrap_or_default();
        if cfg.release_channel != channel {
            cfg.release_channel = channel;
            config::save(&cfg).map_err(|e| format!("persist release_channel: {e}"))?;
            let _ = app.emit("config-changed", &cfg);
        }
    }

    let updater = build_channel_updater(&app, channel)?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let app_for_progress = app.clone();
    let downloaded = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total = std::sync::Arc::new(parking_lot::Mutex::new(None::<u64>));
    let downloaded_for_chunk = std::sync::Arc::clone(&downloaded);
    let total_for_chunk = std::sync::Arc::clone(&total);
    update
        .download_and_install(
            move |chunk_len, content_length| {
                let mut total_lock = total_for_chunk.lock();
                if total_lock.is_none() {
                    *total_lock = content_length;
                }
                let cur = downloaded_for_chunk
                    .fetch_add(chunk_len as u64, std::sync::atomic::Ordering::Relaxed)
                    .saturating_add(chunk_len as u64);
                let _ = app_for_progress.emit(
                    "update-progress",
                    serde_json::json!({
                        "downloaded": cur,
                        "total": *total_lock,
                    }),
                );
            },
            || {
                // download_and_install fires this once the bytes
                // are on disk and the installer is about to run.
                // No-op — the UI already shows "installing" once
                // download completes.
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn save_config(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    cfg: Config,
) -> Result<(), String> {
    use tauri::Emitter;

    // M-T6: the device JWT and org bearer are `#[serde(skip)]`, so the `cfg`
    // arriving from React has both secrets None. Re-hydrate from the keychain
    // before save / cloud-sync so this write isn't seen as a clear and the
    // `device_id` extraction below still works.
    let mut cfg = cfg;
    config::hydrate_secrets(&mut cfg);

    // Capture the previous on-disk state before overwriting, so the
    // cloud-sync transition can compare prev.sync_with_cloud vs next.
    let prev_config = config::load().unwrap_or_default();

    config::save(&cfg).map_err(|e| e.to_string())?;

    // Respawn the sync worker so a toggle of `remote_sync.enabled`
    // (or any other field — URL, token, batch size, etc.) takes
    // effect immediately instead of waiting for the next app start.
    // Idempotent: when the new config disables sync, respawn aborts
    // the old worker and leaves the handle as None.
    sync::respawn(
        Arc::clone(&state.storage),
        Arc::clone(&state.sync_stats),
        Arc::clone(&state.account_status),
        Arc::clone(&state.sync_kick),
        Arc::clone(&state.sync_handle),
        app_handle.clone(),
        Arc::clone(&state.location_catalog),
    );

    // Respawn the org-connector worker so a change to the
    // `org_connector` settings block (enabled toggle, platform_url,
    // bearer_token) takes effect immediately without an app restart.
    org_connector::respawn(
        Arc::clone(&state.storage),
        Arc::clone(&state.location_catalog),
        Arc::clone(&state.org_connector_handle),
        Arc::clone(&state.tail_event_kick),
    );

    // Cloud-sync side-effects: opt-in/out/write-through against the
    // preferences API. Requires a device_id from the JWT; if the token
    // is absent or malformed we skip the transition silently — the
    // on-disk save already succeeded.
    let device_id = match crate::cloud_sync::extract_device_id_from_token(&cfg) {
        Ok(id) => id,
        Err(e) => {
            tracing::debug!(error = %e, "skip cloud-sync transition (no device_id)");
            return Ok(());
        }
    };

    let outcome = crate::cloud_sync::handle_cloud_sync_transition(&prev_config, &cfg, &device_id)
        .await
        .unwrap_or(crate::cloud_sync::TransitionOutcome::NoOp);

    match outcome {
        crate::cloud_sync::TransitionOutcome::Adopted(adopted) => {
            // Re-persist with the adopted remote values and re-emit
            // so the React side re-renders with the merged config.
            if let Err(e) = config::save(&adopted) {
                tracing::warn!(error = %e, "failed to re-persist adopted config");
            } else {
                let _ = app_handle.emit("config-changed", &*adopted);
            }
        }
        crate::cloud_sync::TransitionOutcome::Revoked => {
            // The server says this device may not sync. Flip local
            // toggle off, re-persist, and notify the UI.
            let mut reverted = cfg.clone();
            reverted.sync_with_cloud = false;
            if let Err(e) = config::save(&reverted) {
                tracing::warn!(error = %e, "failed to persist sync-revoked config");
            } else {
                let _ = app_handle.emit("sync-revoked", ());
                let _ = app_handle.emit("config-changed", &reverted);
            }
        }
        _ => {}
    }

    Ok(())
}

/// Apply a named sync-speed preset. The UI calls this when the user
/// flips between Fast / Balanced / Resource-saver — it's a thin
/// convenience over `save_config` that sets the two interval fields
/// to a known-good pair. The preset only touches the interval
/// fields; `priority_event_types`, auth, batch_size, and other
/// settings round-trip unchanged.
///
/// `"custom"` is a no-op marker — picking it tells the UI to expose
/// the raw number inputs; nothing changes server-side until the user
/// edits one of them and the UI calls `save_config` directly. Unknown
/// preset names return an error so a typo in the UI surfaces loudly
/// instead of silently doing nothing.
///
/// Returns the resulting `RemoteSyncConfig` snapshot so the UI can
/// re-render without a follow-up `get_config` round-trip.
#[tauri::command(rename_all = "snake_case")]
pub fn set_sync_preset(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    preset: String,
) -> Result<config::RemoteSyncConfig, String> {
    let mut cfg = config::load().map_err(|e| e.to_string())?;

    // Named presets — chosen to keep the fast/bulk ratio roughly
    // constant (an order of magnitude apart) so a user moving from
    // Fast to Resource-saver still gets timely priority drains
    // relative to bulk on the same schedule.
    match preset.as_str() {
        "fast" => {
            cfg.remote_sync.priority_interval_secs = 3;
            cfg.remote_sync.interval_secs = 30;
        }
        "balanced" => {
            cfg.remote_sync.priority_interval_secs = 5;
            cfg.remote_sync.interval_secs = 60;
        }
        "resource_saver" => {
            cfg.remote_sync.priority_interval_secs = 30;
            cfg.remote_sync.interval_secs = 300;
        }
        "custom" => {
            // No-op: UI will expose number inputs and call
            // save_config when the user commits a value.
            return Ok(cfg.remote_sync);
        }
        other => return Err(format!("unknown sync preset: {other}")),
    }

    config::save(&cfg).map_err(|e| e.to_string())?;
    sync::respawn(
        Arc::clone(&state.storage),
        Arc::clone(&state.sync_stats),
        Arc::clone(&state.account_status),
        Arc::clone(&state.sync_kick),
        Arc::clone(&state.sync_handle),
        app_handle,
        Arc::clone(&state.location_catalog),
    );
    Ok(cfg.remote_sync)
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_discovered_logs() -> Vec<DiscoveredLog> {
    discovery::discover()
}

/// Surface parser coverage to the tray UI: how many lines are
/// recognised, how many were structurally parsed but unclassified,
/// how many were skipped, plus the top 50 unknown event types so
/// the user can see which rules would unlock the most data.
#[tauri::command(rename_all = "snake_case")]
pub fn get_parse_coverage(state: State<'_, AppState>) -> Result<ParseCoverageResponse, String> {
    let stats = state.tail_stats.lock().clone();
    let rows = state
        .storage
        .recent_unknowns(50)
        .map_err(|e| e.to_string())?;
    let unknowns = rows
        .into_iter()
        .map(|r| UnknownSample {
            log_source: r.log_source,
            event_name: r.event_name,
            occurrences: r.occurrences,
            first_seen: r.first_seen,
            last_seen: r.last_seen,
            sample_line: r.sample_line,
            sample_body: r.sample_body,
        })
        .collect();
    Ok(ParseCoverageResponse {
        recognised: stats.events_recognised,
        structural_only: stats.lines_structural_only,
        skipped: stats.lines_skipped,
        noise: stats.lines_noise,
        unknowns,
    })
}

/// Mark an event_name as noise — the next tail drain stops sampling
/// it and the existing unknown sample is dropped immediately. Used by
/// the tray UI's "ignore this" button on the unknowns list.
#[tauri::command(rename_all = "snake_case")]
pub fn mark_event_as_noise(state: State<'_, AppState>, event_name: String) -> Result<(), String> {
    state
        .storage
        .add_noise(&event_name, "user")
        .map_err(|e| e.to_string())
}

// -- Session timeline ------------------------------------------------

/// One row in the player-visible "what happened" feed. The summary is
/// formatted server-side so the frontend stays a thin renderer; if we
/// want to localise later this is the single point we change.
///
/// `raw_line` is the original log line as captured from disk — surfaced
/// by the Logs pane's detail drawer for forensic inspection.
/// `log_source` is the channel tag (LIVE/PTU/EPTU) the event was tailed
/// from, displayed in the drawer's Source row.
/// `synced` is derived (not stored): an event is considered synced
/// when its per-row `sent_at` is a real datetime (sync worker shipped
/// it). NULL means pending in the drain queue; `__quarantined_*`
/// means the poison-pill path shelved it client-side — both surface
/// as `synced = false`. See `synced_from_sent_at`.
#[derive(Debug, Clone, Serialize)]
pub struct TimelineEntry {
    pub id: i64,
    pub timestamp: String,
    pub event_type: String,
    pub summary: String,
    pub raw_line: String,
    pub log_source: String,
    pub synced: bool,
    /// Client-side resolved location for this event, when it carries
    /// one. `None` for placeless events (login, shop, …) or an
    /// unparseable payload. The UI links to `/kb/location/{slug}` when
    /// `slug` is present, else renders `display_name` as plain text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<ResolvedLocation>,
}

/// A location resolved client-side from a raw engine string via the
/// shared classifier. `slug` is present ONLY when the classifier is
/// confident enough to link (an exact or fuzzy catalog hit) — synthetic
/// / heuristic / fallback results carry a friendly `display_name` but
/// no link, per the "best name, link only when confident" rule.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedLocation {
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub tier: starstats_core::location_taxonomy::LocationTier,
}

/// Resolve an event's location (if any) against the catalogue. Pure —
/// no I/O, no state. Returns `None` for placeless events so callers can
/// `?`-chain. The KB link is gated on `slug` being `Some`, which the
/// classifier only sets on a catalog/fuzzy hit.
pub fn resolve_location(
    event: &GameEvent,
    catalog: &starstats_core::location_catalog::LocationCatalog,
) -> Option<ResolvedLocation> {
    let raw = event.location_raw()?;
    let c = starstats_core::location_classifier::classify(raw, catalog);
    Some(ResolvedLocation {
        display_name: c.display_name,
        slug: c.slug,
        system: c.system,
        tier: c.tier,
    })
}

/// Default number of recent events surfaced when the caller doesn't
/// pass a `limit`. Tuned for the StatusPane glance view.
const DEFAULT_TIMELINE_LIMIT: usize = 50;

/// Hard cap on `limit`. Stops a frontend bug from asking for the
/// whole table over IPC — a typical row is ~500 bytes (raw line +
/// payload), so 5000 rows is a ~2.5 MB serialised response, which is
/// the largest we want to ship across the IPC boundary in one call.
const MAX_TIMELINE_LIMIT: usize = 5_000;

/// Clamp a caller-supplied limit into `[1, MAX_TIMELINE_LIMIT]`,
/// substituting the default when `None`. Pulled out of the Tauri
/// command so we can unit-test the bounds without spinning up an
/// AppState.
fn clamp_timeline_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_TIMELINE_LIMIT)
        .clamp(1, MAX_TIMELINE_LIMIT)
}

/// Derive whether an event row has been accepted by the API server,
/// from the per-row `sent_at` column. A bare RFC3339 timestamp means
/// the sync worker successfully shipped the row; a `__quarantined_*`
/// sentinel means the poison-pill path shelved it client-side (the
/// server never accepted it); `NULL` means it's still pending in the
/// drain queue.
///
/// "Synced" in the UI means "the server has it." Quarantined rows
/// have NOT been accepted server-side, so they report `false` here —
/// the user sees them as Pending in the LogsPane and pair with the
/// recovery banner that exposes them.
///
/// Replaces the legacy `r.id <= sync_cursor.last_event_id` check
/// that broke after the priority-lanes refactor stopped advancing
/// the global cursor.
fn synced_from_sent_at(sent_at: Option<&str>) -> bool {
    match sent_at {
        Some(s) => !s.starts_with("__quarantined_"),
        None => false,
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_session_timeline(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<TimelineEntry>, String> {
    let limit = clamp_timeline_limit(limit);
    let rows = state
        .storage
        .recent_events(limit)
        .map_err(|e| e.to_string())?;

    // Snapshot the catalogue once for the whole batch — a cheap `Arc`
    // clone, so we don't re-lock per row.
    let catalog = state.location_catalog.read().clone();

    let entries = rows
        .into_iter()
        .map(|r| {
            // Best-effort summary: if deserialisation fails we still
            // emit a row so the user sees something — they can drill
            // into the raw payload via the inspect tool.
            let (summary, location) = match serde_json::from_str::<GameEvent>(&r.payload_json) {
                Ok(event) => (format_summary(&event), resolve_location(&event, &catalog)),
                Err(_) => (format!("{} (unparseable payload)", r.event_type), None),
            };
            let synced = synced_from_sent_at(r.sent_at.as_deref());
            TimelineEntry {
                id: r.id,
                timestamp: r.timestamp,
                event_type: r.event_type,
                summary,
                raw_line: r.raw_line,
                log_source: r.log_source,
                synced,
                location,
            }
        })
        .collect();

    Ok(entries)
}

/// Page of search hits returned by `search_events`. `total` is the
/// count of all rows matching the predicate (independent of pagination
/// cursor); `has_more` is true iff there exist rows older than the
/// returned page that still match the predicate. The caller can use
/// `total` to show "loaded / total" and `has_more` to decide whether
/// to render a Load-more affordance.
#[derive(Debug, Clone, Serialize)]
pub struct SearchEventsResult {
    pub entries: Vec<TimelineEntry>,
    pub total: u64,
    pub has_more: bool,
}

/// Core implementation of [`search_events`], extracted as a pure
/// function over `&Storage` so unit tests can exercise pagination,
/// has_more, and empty-string normalisation without spinning up a
/// Tauri `State<AppState>`.
fn search_events_impl(
    storage: &crate::storage::Storage,
    query: Option<&str>,
    type_filter: Option<&str>,
    before_id: Option<i64>,
    limit: usize,
) -> Result<SearchEventsResult, String> {
    // Treat empty strings the same as None — saves the front end
    // from having to guard against "" before sending the request.
    let query_ref = query.filter(|s| !s.is_empty());
    let type_ref = type_filter.filter(|s| !s.is_empty());

    let rows = storage
        .search_events_paged(query_ref, type_ref, before_id, limit)
        .map_err(|e| e.to_string())?;
    let total = storage
        .count_matching_events(query_ref, type_ref)
        .map_err(|e| e.to_string())?;

    let entries: Vec<TimelineEntry> = rows
        .into_iter()
        .map(|r| {
            let summary = match serde_json::from_str::<GameEvent>(&r.payload_json) {
                Ok(event) => format_summary(&event),
                Err(_) => format!("{} (unparseable payload)", r.event_type),
            };
            let synced = synced_from_sent_at(r.sent_at.as_deref());
            TimelineEntry {
                id: r.id,
                timestamp: r.timestamp,
                event_type: r.event_type,
                summary,
                raw_line: r.raw_line,
                log_source: r.log_source,
                synced,
                // Logs-search location resolution is wired in a
                // follow-up commit; recent-events (StatusPane) resolves
                // it today. See get_session_timeline.
                location: None,
            }
        })
        .collect();

    // has_more iff the returned page is full AND there exist matching
    // rows older than the smallest id in this page. Computed against
    // `total` so we don't need a follow-up query.
    let returned = entries.len() as u64;
    let has_more = returned == limit as u64 && total > returned;

    Ok(SearchEventsResult {
        entries,
        total,
        has_more,
    })
}

/// Server-side paginated event search. Replaces the LogsPane's
/// previous "fetch 1000 + filter client-side" pattern.
///
/// `query` is a case-insensitive substring matched against `type` and
/// the parsed `payload` JSON. `type_filter`, when present, additionally
/// pins results to exactly that event_type (used by the type-pill row
/// in LogsPane). `before_id` is the cursor for "Load more" — pass the
/// smallest `id` from the current page; pass `None` for the first page.
/// `limit` is clamped by the existing `clamp_timeline_limit`.
#[tauri::command(rename_all = "snake_case")]
pub fn search_events(
    state: State<'_, AppState>,
    query: Option<String>,
    type_filter: Option<String>,
    before_id: Option<i64>,
    limit: Option<usize>,
) -> Result<SearchEventsResult, String> {
    let limit = clamp_timeline_limit(limit);
    search_events_impl(
        &state.storage,
        query.as_deref(),
        type_filter.as_deref(),
        before_id,
        limit,
    )
}

/// How many entries the "Top event types" section is allowed to show.
/// Matches the spec for the clipboard summary — anything past 10 is
/// noise in a Discord paste.
const SESSION_SUMMARY_TOP_TYPES: usize = 10;

/// How many recent timeline rows the summary embeds. Caps the
/// clipboard payload at something hand-scannable.
const SESSION_SUMMARY_TIMELINE_LIMIT: usize = 20;

/// Column width for the event_type cell in the "Top event types"
/// section. Long enough for the longest classifier name we ship
/// without truncation, short enough that the count column stays close.
const SESSION_SUMMARY_TYPE_COL_WIDTH: usize = 22;

/// Column width for the event_type cell in the timeline section. The
/// timeline is denser than the top-types table so the column is
/// narrower; summaries flow into the remaining width.
const SESSION_SUMMARY_TIMELINE_TYPE_COL_WIDTH: usize = 15;

/// Insert a thousands separator into a u64 without pulling in a
/// formatting crate — the summary is the only place we need it.
fn format_count_with_commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

/// Extract `HH:MM` from an RFC3339 timestamp. Falls back to the raw
/// string's first 5 chars if parsing fails, so a malformed value still
/// produces *something* readable rather than the empty cell.
fn timeline_hhmm(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.with_timezone(&chrono::Utc).format("%H:%M").to_string(),
        Err(_) => raw.chars().take(5).collect(),
    }
}

/// Pure formatter for the clipboard-friendly session summary. Kept
/// free of any Tauri state so it can be unit-tested with fixture
/// slices and a pinned `now` instant.
///
/// Layout (sections separated by a blank line):
///   1. Title + "Generated <ts>" header
///   2. "Captured N events total" (or "No events captured yet." short-circuit)
///   3. Top event types (up to 10, padded columns, comma-separated counts)
///   4. Recent timeline (up to 20, HH:MM + padded type + summary)
fn format_session_summary(
    event_counts: &[EventCount],
    timeline: &[TimelineEntry],
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let header_ts = now.format("%Y-%m-%d %H:%M UTC").to_string();
    let total: u64 = event_counts.iter().map(|c| c.count).sum();

    // Empty-state short-circuit. Returning a tiny but still-useful
    // string keeps the clipboard action from looking broken when the
    // store is fresh.
    if total == 0 && timeline.is_empty() {
        return format!(
            "StarStats — session summary\nGenerated {header_ts}\n\nNo events captured yet.\n"
        );
    }

    let mut out = String::new();
    out.push_str("StarStats — session summary\n");
    out.push_str(&format!("Generated {header_ts}\n"));
    out.push('\n');
    out.push_str(&format!(
        "Captured {} events total\n",
        format_count_with_commas(total)
    ));

    if !event_counts.is_empty() {
        out.push('\n');
        out.push_str("Top event types:\n");
        for c in event_counts.iter().take(SESSION_SUMMARY_TOP_TYPES) {
            out.push_str(&format!(
                "  {:<width$}  {}\n",
                c.event_type,
                format_count_with_commas(c.count),
                width = SESSION_SUMMARY_TYPE_COL_WIDTH,
            ));
        }
    }

    if !timeline.is_empty() {
        out.push('\n');
        out.push_str(&format!(
            "Recent timeline (last {}):\n",
            SESSION_SUMMARY_TIMELINE_LIMIT.min(timeline.len())
        ));
        for entry in timeline.iter().take(SESSION_SUMMARY_TIMELINE_LIMIT) {
            out.push_str(&format!(
                "  {}  {:<width$}  {}\n",
                timeline_hhmm(&entry.timestamp),
                entry.event_type,
                entry.summary,
                width = SESSION_SUMMARY_TIMELINE_TYPE_COL_WIDTH,
            ));
        }
    }

    out
}

/// Build a plain-text summary of the current session suitable for
/// pasting into Discord, a forum post, or a bug report. Re-uses the
/// same accessors as `get_status` (event counts) and
/// `get_session_timeline` (recent rows) so the numbers always agree
/// with what the StatusPane is rendering.
///
/// Returns a `String` (not a struct) because the consumer is the
/// clipboard — keeping it pre-formatted on the Rust side avoids
/// scattering layout logic across the JS surface.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_session_summary_text(state: State<'_, AppState>) -> Result<String, String> {
    let counts: Vec<EventCount> = state
        .storage
        .event_counts()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(event_type, count)| EventCount { event_type, count })
        .collect();

    // Pull the same row set get_session_timeline returns, but cap the
    // fetch at the summary's own limit — there's no value spending
    // IPC bandwidth on rows we'd drop anyway.
    let rows = state
        .storage
        .recent_events(SESSION_SUMMARY_TIMELINE_LIMIT)
        .map_err(|e| e.to_string())?;
    let timeline: Vec<TimelineEntry> = rows
        .into_iter()
        .map(|r| {
            let summary = match serde_json::from_str::<GameEvent>(&r.payload_json) {
                Ok(event) => format_summary(&event),
                Err(_) => format!("{} (unparseable payload)", r.event_type),
            };
            let synced = synced_from_sent_at(r.sent_at.as_deref());
            TimelineEntry {
                id: r.id,
                timestamp: r.timestamp,
                event_type: r.event_type,
                summary,
                raw_line: r.raw_line,
                log_source: r.log_source,
                synced,
                // Logs-search location resolution is wired in a
                // follow-up commit; recent-events (StatusPane) resolves
                // it today. See get_session_timeline.
                location: None,
            }
        })
        .collect();

    Ok(format_session_summary(
        &counts,
        &timeline,
        chrono::Utc::now(),
    ))
}

/// Aggregate the recent shop / commodity request-response pairs into
/// transaction rows. Pulls the last `limit` events, deserialises them,
/// hands the slice to `starstats_core::pair_transactions`, and returns
/// the resulting `Vec<Transaction>` to JS.
///
/// `window_secs` is the "if we haven't seen a response in N seconds,
/// mark it timed out" threshold. 30s is the default the UI uses; the
/// param exists so debugging can dial it down.
#[tauri::command(rename_all = "snake_case")]
pub fn list_transactions(
    state: State<'_, AppState>,
    limit: Option<usize>,
    window_secs: Option<i64>,
) -> Result<Vec<Transaction>, String> {
    let limit = clamp_timeline_limit(limit);
    let rows = state
        .storage
        .recent_events(limit)
        .map_err(|e| e.to_string())?;
    let events: Vec<GameEvent> = rows
        .into_iter()
        .filter_map(|r| serde_json::from_str::<GameEvent>(&r.payload_json).ok())
        .collect();
    // `now` for the ageing clock is the system time in UTC ISO. We
    // don't pull `chrono` here because we're already on it via the
    // workspace dep — `to_rfc3339()` matches the format the parser
    // emits.
    let now = chrono::Utc::now().to_rfc3339();
    let window = window_secs.unwrap_or(30);
    Ok(pair_transactions(&events, &now, window))
}

/// Aggregate counters surfaced by the Logs pane's headline strip:
/// how many events live in the local store and how big the on-disk
/// SQLite file currently is. Cheap to compute (two pragmas + a count)
/// and pulled on the same 10s cadence as the timeline.
#[derive(Debug, Clone, Serialize)]
pub struct StorageStats {
    pub total_events: u64,
    pub db_size_bytes: u64,
}

/// Combined snapshot of every secondary-source pipeline. Surfaced as
/// one command so the StatusPane can render a single "Sources" card
/// without three round-trips. Each sub-stats struct lives next to
/// its module — this is just the wire envelope.
#[derive(Debug, Clone, Serialize)]
pub struct SourceStats {
    pub launcher: LauncherStats,
    pub crashes: CrashStats,
    pub backfill: BackfillStats,
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_source_stats(state: State<'_, AppState>) -> SourceStats {
    SourceStats {
        launcher: state.launcher_stats.lock().clone(),
        crashes: state.crash_stats.lock().clone(),
        backfill: state.backfill_stats.lock().clone(),
    }
}

/// Marketing-version string (Cargo.toml workspace version), surfaced
/// to the UI so the displayed version matches GitHub release tags
/// (e.g. "0.2.0-alpha"). This is distinct from Tauri's `getVersion()`
/// API, which returns the numeric `tauri.conf.json` version (MSI
/// bundlers reject non-numeric pre-release identifiers, so the Tauri
/// version is intentionally a numeric subset of the marketing one).
#[tauri::command(rename_all = "snake_case")]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Returns the release channel of the *running* binary, parsed from
/// `CARGO_PKG_VERSION`. Used by the tray Settings UI to detect a
/// mismatch between the build's channel and the user's configured
/// `release_channel`, and surface the channel-mismatch banner.
///
/// This is the "build channel" leg of the two-channel model the
/// Updates card surfaces — distinct from `Config::release_channel`,
/// which is the user's *preferred* update channel. They diverge when
/// a user installs an off-channel binary (e.g. side-loading a beta
/// while still configured for Live).
#[tauri::command(rename_all = "snake_case")]
pub fn get_build_release_channel() -> crate::config::ReleaseChannel {
    crate::config::ReleaseChannel::from_version(env!("CARGO_PKG_VERSION"))
}

/// Result of a re-parse pass over the local store.
#[derive(Debug, Clone, Serialize)]
pub struct ReparseStats {
    pub examined: u64,
    /// Rows whose `(type, payload)` changed because a newer/remote
    /// rule produced a different classification.
    pub updated: u64,
    /// Rows whose stored line no longer parses (probably mid-flight
    /// at capture time). Left untouched — never demoted.
    pub kept_unmatched: u64,
    /// Unknowns whose sample line now classifies. The first occurrence
    /// of each is promoted into `events`; the unknown row is removed.
    pub promoted_unknowns: u64,
    /// Bursts retroactively detected over already-stored events. Each
    /// hit produces one `burst_summary` row; the original member rows
    /// are deleted. Sessions already collapsed at live-tail time are a
    /// no-op here because the idempotency key matches the live shape.
    pub bursts_collapsed: u64,
    /// Total per-line member rows deleted as part of `bursts_collapsed`.
    /// Surfaced separately so the user can see the spam-reduction effect
    /// (a single burst commonly absorbs 20+ rows).
    pub members_suppressed: u64,
    pub error: Option<String>,
}

/// Re-run the current classifier (built-ins + body-prefix + remote
/// rules) over every stored event line, in place. Existing rows are
/// updated when the new classification differs; otherwise left alone.
/// Idempotent — running it twice with the same rule set is a no-op
/// past the first.
///
/// Also walks `unknown_event_samples` and promotes any sample line
/// that the current classifier now recognises into a real `events`
/// row, removing the unknown record.
///
/// Heavy operation — async + spawn_blocking so the webview stays
/// responsive on a multi-million-row store.
#[tauri::command(rename_all = "snake_case")]
pub async fn reparse_events(state: State<'_, AppState>) -> Result<ReparseStats, String> {
    let storage = Arc::clone(&state.storage);
    let rules_snapshot = state.parser_def_cache.snapshot();

    tauri::async_runtime::spawn_blocking(move || run_reparse(&storage, &rules_snapshot))
        .await
        .map_err(|e| format!("reparse worker panicked: {e}"))?
}

/// Wall-clock ceiling (seconds) between consecutive burst members during
/// the Re-parse retro-burst scan. Members further apart than this end the
/// burst, so runs from different sessions — adjacent in the offset-sorted
/// window after a Game.log rotation resets `source_offset` — never weld
/// into one summary (H1). 120s comfortably covers a legitimate loadout
/// re-equip while sitting far below any inter-session gap.
const RETRO_BURST_MAX_MEMBER_GAP_SECS: i64 = 120;

fn run_reparse(
    storage: &crate::storage::Storage,
    rules: &[starstats_core::CompiledRemoteRule],
) -> Result<ReparseStats, String> {
    let mut stats = ReparseStats {
        examined: 0,
        updated: 0,
        kept_unmatched: 0,
        promoted_unknowns: 0,
        bursts_collapsed: 0,
        members_suppressed: 0,
        error: None,
    };

    // Phase 1 — re-classify already-recognised events. Also tracks
    // the most-recent zone signal as we walk so death events can be
    // back-filled with a best-effort `zone` field.
    //
    // Walk order is `id ASC` (per for_each_event), which matches
    // ingest order. For the typical workflow — live tail + on-startup
    // backfill of rotated logs — that approximates timestamp order
    // closely enough for the enrichment to land the right zone on
    // each death. Edge case: a late backfill that ingests OLDER logs
    // AFTER newer live-tail events would attribute the wrong zone
    // to those older deaths; users can re-run Re-parse after the
    // backfill catches up to fix it.
    let mut zone_tracker = ZoneTracker::default();
    let outcome = storage.for_each_event(500, |row| {
        stats.examined += 1;
        let Some(parsed) = structural_parse(&row.raw_line) else {
            stats.kept_unmatched += 1;
            return Ok(());
        };
        let Some(new_event) = classify(&parsed).or_else(|| apply_remote_rules(&parsed, rules))
        else {
            // The current rule set produces nothing for this line;
            // never demote — the row was recognised previously and
            // its stored payload is the best record we have.
            stats.kept_unmatched += 1;
            return Ok(());
        };

        // Update the zone tracker BEFORE enriching, so a death event
        // co-located with a fresh PlanetTerrainLoad on the same tick
        // doesn't accidentally pick up the older zone. The source
        // idempotency_key is captured alongside the zone so the
        // enrichment can record per-field provenance pointing back
        // at the row that contributed the value.
        match &new_event {
            GameEvent::PlanetTerrainLoad(t) => {
                zone_tracker.observe(t.planet.clone(), row.idempotency_key.clone());
            }
            GameEvent::LocationInventoryRequested(l) if l.location != "INVALID_LOCATION_ID" => {
                zone_tracker.observe(l.location.clone(), row.idempotency_key.clone());
            }
            _ => {}
        }

        // Best-effort zone enrichment for death-related events.
        // Classify always returns `zone: None`; the enrichment pass
        // injects whatever zone_tracker has accumulated and stamps
        // `metadata.field_provenance["zone"]` via
        // `starstats_core::provenance_for_inferred_field` so the
        // inference trail is preserved alongside the value.
        let (new_event, zone_filled_from) = match new_event {
            GameEvent::PlayerDeath(mut d) if d.zone.is_none() => {
                let source_key = zone_tracker.fill(&mut d.zone);
                (GameEvent::PlayerDeath(d), source_key)
            }
            GameEvent::PlayerIncapacitated(mut i) if i.zone.is_none() => {
                let source_key = zone_tracker.fill(&mut i.zone);
                (GameEvent::PlayerIncapacitated(i), source_key)
            }
            other => (other, None),
        };

        let Some((new_type, new_ts, new_payload)) = serialise_for_reparse(&new_event) else {
            stats.kept_unmatched += 1;
            return Ok(());
        };
        let new_metadata_json = build_zone_metadata_json(&new_event, zone_filled_from.as_deref());
        // Re-write only when something actually changed. Metadata
        // dirties the row even if the GameEvent payload itself is
        // unchanged from a previous run — we want the provenance
        // stamped on every re-parse pass where enrichment fired.
        let payload_dirty = new_type != row.event_type
            || new_ts != row.timestamp
            || new_payload != row.payload_json;
        let metadata_dirty = new_metadata_json.is_some()
            && new_metadata_json.as_deref() != row.metadata_json.as_deref();
        if payload_dirty || metadata_dirty {
            storage
                .update_event_classification(
                    row.id,
                    &new_type,
                    &new_ts,
                    &new_payload,
                    new_metadata_json.as_deref(),
                )
                .map_err(|e| anyhow::anyhow!("update_event_classification: {e}"))?;
            stats.updated += 1;
        }
        Ok(())
    });
    if let Err(e) = outcome {
        stats.error = Some(format!("phase 1: {e}"));
        return Ok(stats);
    }

    // Phase 2 — promote unknowns whose stored sample now classifies.
    let unknowns = storage
        .recent_unknowns(usize::MAX)
        .map_err(|e| format!("recent_unknowns: {e}"))?;
    for sample in unknowns {
        let Some(parsed) = structural_parse(&sample.sample_line) else {
            continue;
        };
        let Some(new_event) = classify(&parsed).or_else(|| apply_remote_rules(&parsed, rules))
        else {
            continue;
        };
        let Some((new_type, new_ts, new_payload)) = serialise_for_reparse(&new_event) else {
            continue;
        };
        // We don't know the original byte offset for the unknown, so
        // synthesise a key keyed on the sample line itself. ON CONFLICT
        // DO NOTHING means a duplicate (same line in events already)
        // is silently skipped; success means a real promotion.
        let key = reparse_idempotency_key(&sample.log_source, &sample.sample_line);
        let insert_outcome = storage.insert_event(
            &key,
            &new_type,
            &new_ts,
            &sample.sample_line,
            &new_payload,
            &sample.log_source,
            0,
        );
        if let Err(e) = insert_outcome {
            tracing::warn!(error = %e, event = %sample.event_name, "promote unknown failed");
            continue;
        }
        // Remove the unknown sample regardless of whether the insert
        // was a fresh row or a no-op conflict — either way, the
        // sample is no longer a "next thing to write a rule for".
        if let Err(e) = storage.delete_unknown(&sample.log_source, &sample.event_name) {
            tracing::warn!(error = %e, "delete_unknown failed during reparse");
        }
        stats.promoted_unknowns += 1;
    }

    // Phase 3 — retro-burst detection. Walk each `log_source`'s history
    // in source-offset order, run `detect_bursts` over the
    // structural-parsed view, and replace matched runs with a single
    // synthetic `BurstSummary` row plus member deletions. The
    // idempotency key matches the live-tail format (UUIDv5 over
    // `log_source : anchor_offset : "{raw_line}|burst:{rule_id}:{size}"`)
    // so a session that was already collapsed live can never produce a
    // duplicate summary, and re-running this phase is a strict no-op
    // once the members have been deleted.
    let burst_rules = crate::burst_rules::builtin_burst_rules();
    let sources = match storage.distinct_log_sources() {
        Ok(s) => s,
        Err(e) => {
            stats.error = Some(format!("phase 3: distinct_log_sources: {e}"));
            return Ok(stats);
        }
    };
    for source in sources {
        let rows = match storage.events_for_burst_scan(&source) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, log_source = %source, "phase 3: events_for_burst_scan");
                continue;
            }
        };
        if rows.is_empty() {
            continue;
        }
        // Project to (parseable_index → row_idx) so detect_bursts sees a
        // contiguous LogLine stream without holes from corrupt or
        // truncated raw lines. Also skip already-collapsed
        // `burst_summary` rows from a previous pass — re-parsing them
        // would just no-op anyway, but skipping is cheaper than
        // re-running detect_bursts over them.
        let parsed: Vec<(usize, LogLine<'_>)> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.event_type != "burst_summary")
            .filter_map(|(idx, r)| structural_parse(&r.raw_line).map(|l| (idx, l)))
            .collect();
        if parsed.len() < 2 {
            continue;
        }
        let log_lines: Vec<LogLine<'_>> = parsed.iter().map(|(_, l)| l.clone()).collect();
        // The retro-burst window is ordered by `source_offset`, which
        // resets to 0 on every Game.log rotation — so attachment runs from
        // different sessions at overlapping offsets sit adjacent here. A
        // wall-clock ceiling stops `detect_bursts` welding them into one
        // summary and destructively deleting the cross-session members (H1).
        let hits =
            detect_bursts_with_time_gap(&log_lines, &burst_rules, RETRO_BURST_MAX_MEMBER_GAP_SECS);

        for hit in hits {
            // Map BurstHit indices (into `log_lines`) back to row
            // positions, then back to a BurstScanRow ref for the anchor
            // (the only one we need a stable id/offset/raw_line for —
            // the end position contributes only its timestamp via
            // `end_log` below).
            let anchor_row_idx = parsed[hit.start_index].0;
            let anchor_row = &rows[anchor_row_idx];
            let anchor_log = &log_lines[hit.start_index];
            let end_log = &log_lines[hit.end_index];
            let member_db_ids: Vec<i64> = hit
                .member_indices
                .iter()
                .map(|&i| rows[parsed[i].0].id)
                .collect();

            // Cap the anchor body before storing so a 20-page inventory
            // dump doesn't end up in the timeline payload. Matches the
            // 200-char cap in `process_buffer`.
            let sample: String = anchor_log.body.chars().take(200).collect();

            // For loadout_restore bursts, classify each member line to
            // extract item_class values for the web loadout widget.
            // NOTE: the rule id is LOADOUT_RESTORE_BURST_RULE_ID ("loadout_restore_burst");
            // the bare "loadout_restore" here was the bug that caused the re-parse path to
            // always skip this branch, leaving the burst with kind=None and no categories.
            let (burst_kind, burst_categories, burst_items) = if hit.rule_id
                == crate::burst_rules::LOADOUT_RESTORE_BURST_RULE_ID
            {
                let pairs: Vec<(String, String)> = hit
                    .member_indices
                    .iter()
                    .filter_map(|&vi| {
                        if let Some(GameEvent::AttachmentReceived(ar)) = classify(&log_lines[vi]) {
                            Some((ar.item_class, ar.port))
                        } else {
                            None
                        }
                    })
                    .collect();
                let categories = build_loadout_categories(
                    &pairs.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>(),
                );
                let items = build_loadout_items(&pairs);
                (Some("loadout_restore".to_string()), categories, items)
            } else {
                (None, None, None)
            };

            let summary = GameEvent::BurstSummary(BurstSummary {
                timestamp: anchor_log.timestamp.to_string(),
                rule_id: hit.rule_id.clone(),
                size: hit.size as u32,
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

            let Some((event_type, ts, payload)) = serialise_for_reparse(&summary) else {
                tracing::warn!(rule = %hit.rule_id, "phase 3: serialise BurstSummary");
                continue;
            };

            let synthetic_line =
                format!("{}|burst:{}:{}", anchor_row.raw_line, hit.rule_id, hit.size);
            let key = burst_idempotency_key(&source, anchor_row.source_offset, &synthetic_line);

            if let Err(e) = storage.insert_event(
                &key,
                &event_type,
                &ts,
                &anchor_row.raw_line,
                &payload,
                &source,
                anchor_row.source_offset,
            ) {
                tracing::warn!(error = %e, rule = %hit.rule_id, "phase 3: insert burst summary");
                continue;
            }

            // Delete each member row. The summary itself was inserted
            // under a fresh idempotency key (different from any
            // member's), so deleting members can never delete the
            // summary we just wrote.
            for id in &member_db_ids {
                match storage.delete_event_by_id(*id) {
                    Ok(n) => {
                        stats.members_suppressed = stats.members_suppressed.saturating_add(n as u64)
                    }
                    Err(e) => tracing::warn!(error = %e, id = id, "phase 3: delete member"),
                }
            }
            stats.bursts_collapsed += 1;
        }
    }

    Ok(stats)
}

/// Idempotency key for a retro-emitted burst summary. Same shape as
/// `gamelog::idempotency_key` (UUIDv5 over `source:offset:line`) so a
/// session that was already collapsed at live-tail time produces an
/// identical key and the `ON CONFLICT DO NOTHING` clause keeps the
/// existing row instead of inserting a duplicate.
fn burst_idempotency_key(log_source: &str, offset: u64, synthetic_line: &str) -> String {
    use uuid::Uuid;
    let payload = format!("{log_source}:{offset}:{synthetic_line}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, payload.as_bytes()).to_string()
}

/// Outcome of `reingest_rotated_logs`. Mirrors the on-disk shape of
/// `BackfillStats` but without `completed`/`files_already_done` —
/// this command always re-walks every archived file from offset 0
/// regardless of saved cursor state, so those flags don't apply.
#[derive(Debug, Clone, Serialize)]
pub struct ReingestStats {
    pub files_walked: u32,
    pub files_failed: u32,
    pub lines_processed: u64,
    pub events_recognised: u64,
    /// Final non-fatal error message if the walk aborted partway.
    /// `None` on a clean run.
    pub error: Option<String>,
}

/// Forces a full re-walk of every rotated `Game-*.log` file, ignoring
/// the saved per-file cursor. Each line is fed back through
/// `ingest_one_line`, which dedupes already-known events via the
/// `(log_source, line_offset, line)` idempotency key — so previously-
/// classified rows stay where they are and only NEW classifications
/// (e.g. body-line PlayerDeath events under v0.3.2+ that were `None`'d
/// by an older parser) land fresh.
///
/// Side effect: `unknown_event_samples.occurrences` will inflate for
/// any event_name that still doesn't classify, because record_unknown
/// re-bumps the count on each pass. Acceptable noise — the goal is
/// recovering historical events that the modern parser now handles.
///
/// After the walk completes, the cursor is rewritten to EOF so the
/// next startup backfill short-circuits. The user typically clicks
/// Re-parse next to back-fill zone enrichment on the new rows.
#[tauri::command(rename_all = "snake_case")]
pub async fn reingest_rotated_logs(state: State<'_, AppState>) -> Result<ReingestStats, String> {
    let storage = Arc::clone(&state.storage);
    let rules_snapshot = state.parser_def_cache.snapshot();
    // Read the feature flag once at command entry so the worker sees a
    // consistent value for the entire walk. Falling back to `false` on
    // a load error matches the global "default off" contract for the
    // v2 pipeline.
    let enable_v2_metadata = crate::config::load()
        .map(|c| c.v2_metadata_enabled())
        .unwrap_or(false);
    // Re-ingest replays the user's OWN logs, so the claimed handle is the
    // right own-handle to redact from any unknown lines captured here.
    // (Distinct from the event row's `claimed_handle`, which stays server-
    // derived — this value is only the PII redaction input.)
    let own_handle = crate::config::load()
        .ok()
        .and_then(|c| c.remote_sync.claimed_handle.clone())
        .unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        run_reingest(&storage, &rules_snapshot, enable_v2_metadata, &own_handle)
    })
    .await
    .map_err(|e| format!("reingest worker panicked: {e}"))?
}

fn run_reingest(
    storage: &crate::storage::Storage,
    rules: &[starstats_core::CompiledRemoteRule],
    enable_v2_metadata: bool,
    own_handle: &str,
) -> Result<ReingestStats, String> {
    use crate::discovery::{self, LogKind};
    use crate::gamelog::{
        file_signature_sync, ingest_one_line, log_source_enum_from_str, log_source_from_path,
        IngestOutcome,
    };
    use std::fs::File;
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let archived: Vec<_> = discovery::discover()
        .into_iter()
        .filter(|d| d.kind == LogKind::ChannelArchived)
        .collect();

    let mut stats = ReingestStats {
        files_walked: 0,
        files_failed: 0,
        lines_processed: 0,
        events_recognised: 0,
        error: None,
    };

    for log in archived {
        let path_str = log.path.to_string_lossy().to_string();
        let log_source = log_source_from_path(&log.path);
        let log_source_enum = log_source_enum_from_str(&log_source);

        let mut file = match File::open(&log.path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(
                    path = %log.path.display(),
                    error = %e,
                    "reingest: open failed",
                );
                stats.files_failed += 1;
                continue;
            }
        };

        // Salt with the same signature the live tail / backfill used for
        // these bytes so re-ingested events dedupe against the existing
        // rows instead of duplicating them (F1). Rotation-by-rename
        // preserves creation time + head, so the archive signs identically
        // to when it was the active Game.log.
        let file_sig = file
            .metadata()
            .ok()
            .and_then(|m| file_signature_sync(&mut file, &m).ok().flatten());
        // `file_signature_sync` leaves the cursor past the head; reingest
        // reads from offset 0, so rewind before wrapping in BufReader.
        if let Err(e) = file.seek(SeekFrom::Start(0)) {
            tracing::warn!(
                path = %log.path.display(),
                error = %e,
                "reingest: rewind failed",
            );
            stats.files_failed += 1;
            continue;
        }

        let mut reader = BufReader::new(file);
        let mut offset: u64 = 0;
        let mut line_buf = String::new();
        let mut local_lines: u64 = 0;
        let mut local_events: u64 = 0;

        loop {
            let line_start = offset;
            line_buf.clear();
            let n = match reader.read_line(&mut line_buf) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(
                        path = %log.path.display(),
                        error = %e,
                        "reingest: read_line failed; stopping this file",
                    );
                    break;
                }
            };
            if n == 0 {
                break;
            }
            if !line_buf.ends_with('\n') {
                // Truncated final line — skip it like backfill does.
                break;
            }
            offset += n as u64;
            local_lines += 1;
            let outcome = ingest_one_line(
                line_buf.trim_end_matches(['\r', '\n']),
                storage,
                &log_source,
                log_source_enum,
                line_start,
                file_sig.as_deref(),
                rules,
                enable_v2_metadata,
                own_handle,
                None,
            );
            if matches!(outcome, IngestOutcome::Recognised { .. }) {
                local_events += 1;
            }
        }

        // Park the cursor at EOF so the next startup backfill skips
        // this file. The whole point of this command is to bypass the
        // cursor, but ONLY for this run; subsequent startups should
        // resume the normal short-circuit path.
        if let Err(e) = storage.write_cursor(&path_str, offset) {
            tracing::warn!(
                path = %log.path.display(),
                error = %e,
                "reingest: write_cursor failed",
            );
        }
        stats.files_walked += 1;
        stats.lines_processed = stats.lines_processed.saturating_add(local_lines);
        stats.events_recognised = stats.events_recognised.saturating_add(local_events);
    }

    Ok(stats)
}

/// Mirror of `gamelog::serialise_event` but private to the reparse
/// path so tweaks here don't ripple into ingest. Returns
/// `(event_type, timestamp, payload_json)`.
fn serialise_for_reparse(event: &GameEvent) -> Option<(String, String, String)> {
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

/// Most-recent zone signal seen during a reparse walk, paired with the
/// idempotency_key of the source event that contributed it. Captured
/// alongside the zone so the enrichment pass can stamp
/// `metadata.field_provenance["zone"]` with a trail back to the
/// originating `PlanetTerrainLoad` / `LocationInventoryRequested` row.
#[derive(Debug, Default, Clone)]
struct ZoneTracker {
    zone: Option<String>,
    source_idempotency_key: Option<String>,
}

impl ZoneTracker {
    /// Record a fresh zone signal. Overwrites any previous value —
    /// reparse walks ingest order, so the latest signal is the one
    /// we want to attribute to subsequent death events.
    fn observe(&mut self, zone: String, source_idempotency_key: String) {
        self.zone = Some(zone);
        self.source_idempotency_key = Some(source_idempotency_key);
    }

    /// Best-effort enrichment: copy the tracked zone into `target` and
    /// return the source idempotency_key, so the caller can build a
    /// `FieldProvenance::InferredFrom` entry. Returns `None` when the
    /// tracker hasn't seen a zone signal yet — the target is left
    /// untouched in that case.
    fn fill(&self, target: &mut Option<String>) -> Option<String> {
        let zone = self.zone.clone()?;
        *target = Some(zone);
        self.source_idempotency_key.clone()
    }
}

/// Build the JSON-encoded `EventMetadata` blob to persist alongside a
/// re-parsed event row. Returns `None` for events the enrichment pass
/// did not touch — those rows keep whatever metadata they had before.
///
/// When the zone was filled from an upstream signal, the returned
/// metadata anchors at `EventSource::Observed` (the death event itself
/// was observed in the log; only the `zone` field is derived) and
/// stamps `field_provenance["zone"]` via
/// [`starstats_core::provenance_for_inferred_field`].
///
/// `claimed_handle` is `None` here because re-parse runs without
/// per-row user context; the server's ingest path re-stamps the
/// envelope with the correct claimed handle before persisting.
fn build_zone_metadata_json(event: &GameEvent, zone_filled_from: Option<&str>) -> Option<String> {
    let source = zone_filled_from?;
    let mut meta = starstats_core::metadata::stamp(event, None);
    meta.field_provenance.insert(
        "zone".to_string(),
        starstats_core::provenance_for_inferred_field("zone", &[source]),
    );
    serde_json::to_string(&meta).ok()
}

/// Stable key for an unknown-promoted-during-reparse row. Distinct
/// namespace (`reparse:`) so it can never collide with the live-tail
/// keyspace (`<source>:<offset>:<line>`) — same line + same source
/// produces the same key, so re-running reparse is idempotent.
fn reparse_idempotency_key(log_source: &str, line: &str) -> String {
    use uuid::Uuid;
    let payload = format!("reparse:{log_source}:{line}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, payload.as_bytes()).to_string()
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_storage_stats(state: State<'_, AppState>) -> Result<StorageStats, String> {
    let total_events = state.storage.total_events().map_err(|e| e.to_string())?;
    let db_size_bytes = state
        .storage
        .database_size_bytes()
        .map_err(|e| e.to_string())?;
    Ok(StorageStats {
        total_events,
        db_size_bytes,
    })
}

/// Per-variant pretty rendering for the timeline. Kept exhaustive so
/// adding a new GameEvent variant fails to compile without an explicit
/// summary — the compiler is the safety net for "did we forget to
/// surface this in the UI".
fn format_summary(event: &GameEvent) -> String {
    match event {
        GameEvent::ProcessInit(_) => "Game process started".to_string(),
        GameEvent::LegacyLogin(e) => format!("Logged in as {}", e.handle),
        GameEvent::JoinPu(e) => format!("Joined PU shard {} ({}:{})", e.shard, e.address, e.port),
        GameEvent::ChangeServer(e) => format!(
            "Server transition: {}",
            match e.phase {
                starstats_core::ServerPhase::Start => "starting",
                starstats_core::ServerPhase::End => "complete",
            }
        ),
        GameEvent::SeedSolarSystem(e) => format!("Seeded {} on shard {}", e.solar_system, e.shard),
        GameEvent::ResolveSpawn(e) => format!(
            "Spawn resolved (player {}, fallback={})",
            e.player_geid, e.fallback
        ),
        GameEvent::ActorDeath(e) => format!(
            "{} killed by {} ({}, {})",
            e.victim, e.killer, e.weapon, e.damage_type
        ),
        GameEvent::PlayerDeath(e) => {
            // Strip the leading `body_` so the body class reads as
            // a recognisable variant name (e.g. `01_noMagicPocket`)
            // rather than redundant prefix noise.
            let class = e.body_class.strip_prefix("body_").unwrap_or(&e.body_class);
            match &e.zone {
                Some(z) => format!("Died ({class}) in {z}"),
                None => format!("Died ({class})"),
            }
        }
        GameEvent::PlayerIncapacitated(e) => match &e.zone {
            Some(z) => format!("Incapacitated in {z}"),
            None => "Incapacitated".to_string(),
        },
        GameEvent::VehicleDestruction(e) => format!(
            "Vehicle destroyed: {} (level {}, by {})",
            e.vehicle_class, e.destroy_level, e.caused_by
        ),
        GameEvent::HudNotification(e) => {
            // Trim the colon-space the engine pads onto banner text.
            let text = e.text.trim_end_matches(": ").trim_end_matches(':');
            format!("HUD: {text}")
        }
        GameEvent::LocationInventoryRequested(e) => {
            if e.location == "INVALID_LOCATION_ID" {
                format!("{} opened inventory (no location bound yet)", e.player)
            } else {
                format!("{} opened inventory at {}", e.player, e.location)
            }
        }
        GameEvent::PlanetTerrainLoad(e) => {
            // Strip the OOC_<system>_<key>_ prefix so we surface the
            // human-recognisable name (Daymar, Hurston, ArcCorp, etc.).
            let label = e.planet.rsplit('_').next().unwrap_or(&e.planet);
            format!("Near planet/moon: {label}")
        }
        GameEvent::QuantumTargetSelected(e) => {
            let phase = match e.phase {
                starstats_core::QuantumTargetPhase::FuelRequested => "fuel calc",
                starstats_core::QuantumTargetPhase::Selected => "selected",
            };
            format!(
                "Quantum target {phase}: {} → {}",
                e.vehicle_class, e.destination
            )
        }
        GameEvent::MissionQuantumDestinationSelected(e) => {
            format!("Mission destination selected: beacon {}", e.beacon_id)
        }
        GameEvent::TravelToContractLocation(e) => {
            format!("Intends to travel to contract beacon {}", e.beacon_id)
        }
        GameEvent::AttachmentReceived(e) => format!("Attached {} to {}", e.item_class, e.port),
        GameEvent::VehicleStowed(e) => {
            // Drop the `LandingArea_` / `[PROC]LandingArea_` prefix
            // so the surface area is readable.
            let area = e
                .landing_area
                .trim_start_matches("[PROC]")
                .trim_start_matches("LandingArea_");
            format!("Ship {} stowed at {}", e.vehicle_id, area)
        }
        GameEvent::GameCrash(e) => {
            // Use the dir name itself in the summary — it doubles as
            // a human-readable timestamp for crashes whose folder
            // followed the YYYY-MM-DD-HH-MM-SS convention.
            format!("Game crash ({}, {})", e.channel, e.crash_dir_name)
        }
        GameEvent::LauncherActivity(e) => {
            // Launcher messages are free-form. Truncate aggressively
            // for the timeline summary so a paragraph-long error
            // doesn't blow out the row height — the detail drawer
            // still surfaces the full body. The classified category
            // (auth/install/patch/...) leads so a glance shows what
            // the launcher is doing without reading the body.
            const SUMMARY_MAX: usize = 72;
            let truncated: String = e.message.chars().take(SUMMARY_MAX).collect();
            let suffix = if e.message.chars().count() > SUMMARY_MAX {
                "…"
            } else {
                ""
            };
            let category = match e.category {
                starstats_core::LauncherCategory::Auth => "AUTH",
                starstats_core::LauncherCategory::Install => "INSTALL",
                starstats_core::LauncherCategory::Patch => "PATCH",
                starstats_core::LauncherCategory::Update => "UPDATE",
                starstats_core::LauncherCategory::Error => "ERROR",
                starstats_core::LauncherCategory::Info => "INFO",
            };
            format!("[{category}] {truncated}{suffix}")
        }
        GameEvent::MissionStart(e) => {
            let kind = match e.marker_kind {
                starstats_core::MissionMarkerKind::Phase => "Mission accepted",
                starstats_core::MissionMarkerKind::Objective => "Mission objective",
            };
            // Mission name when the engine carried it; otherwise fall
            // back to the bare id so timeline rows stay distinguishable.
            let label = e.mission_name.as_deref().unwrap_or(&e.mission_id);
            format!("{kind}: {label}")
        }
        GameEvent::MissionEnd(e) => {
            // Outcome is best-effort; if missing, just record that the
            // mission terminated. Pair with a prior MissionStart for
            // duration if needed.
            match (&e.outcome, &e.mission_id) {
                (Some(o), _) => format!("Mission ended ({o})"),
                (None, Some(id)) => format!("Mission ended ({id})"),
                (None, None) => "Mission ended".to_string(),
            }
        }
        GameEvent::ShopBuyRequest(e) => match (&e.item_class, &e.quantity) {
            (Some(item), Some(qty)) => format!("Shop buy: {item} x{qty}"),
            (Some(item), None) => format!("Shop buy: {item}"),
            (None, _) => "Shop buy (pending)".to_string(),
        },
        GameEvent::ShopFlowResponse(e) => match e.success {
            Some(true) => "Shop purchase confirmed".to_string(),
            Some(false) => "Shop purchase rejected".to_string(),
            None => "Shop response".to_string(),
        },
        GameEvent::CommodityBuyRequest(e) => match (&e.commodity, &e.quantity) {
            (Some(c), Some(q)) => format!("Commodity buy: {c} ({q})"),
            (Some(c), None) => format!("Commodity buy: {c}"),
            (None, _) => "Commodity buy (pending)".to_string(),
        },
        GameEvent::CommoditySellRequest(e) => match (&e.commodity, &e.quantity) {
            (Some(c), Some(q)) => format!("Commodity sell: {c} ({q})"),
            (Some(c), None) => format!("Commodity sell: {c}"),
            (None, _) => "Commodity sell (pending)".to_string(),
        },
        GameEvent::SessionEnd(e) => match e.kind {
            starstats_core::SessionEndKind::SystemQuit => "Session ended (clean quit)".to_string(),
            starstats_core::SessionEndKind::FastShutdown => {
                "Session ended (fast shutdown)".to_string()
            }
        },
        GameEvent::RemoteMatch(e) => {
            // Show the rule's declared event name + a compact field
            // peek so the user can tell rules apart at a glance. We
            // don't try to reconstruct natural-language summaries —
            // rule authors don't know our format and we'd just be
            // making up text. The detail drawer renders fields fully.
            if e.fields.is_empty() {
                format!("[remote] {}", e.event_name)
            } else {
                let preview: Vec<String> = e
                    .fields
                    .iter()
                    .take(2)
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                format!("[remote] {} ({})", e.event_name, preview.join(", "))
            }
        }
        GameEvent::BurstSummary(e) => {
            // Friendlier rendering for the four built-in rules; falls
            // back to a generic "Burst: <id>" for anything else (e.g.
            // future remote-served rules).
            let label = match e.rule_id.as_str() {
                "loadout_restore_burst" => "Loadout restored",
                "terrain_load_burst" => "Terrain loaded",
                "hud_notification_burst" => "Notifications",
                "vehicle_stowed_burst" => "Vehicles stowed",
                _ => "Burst",
            };
            format!("{} ({} events)", label, e.size)
        }
        GameEvent::LocationChanged(e) => match &e.from {
            Some(from) => format!("Location: {} → {}", from, e.to),
            None => format!("Location: {}", e.to),
        },
        GameEvent::ShopRequestTimedOut(e) => match &e.item_class {
            Some(item) => format!(
                "Shop request timed out: {item} (after {}s)",
                e.timed_out_after_secs
            ),
            None => format!("Shop request timed out (after {}s)", e.timed_out_after_secs),
        },
        GameEvent::MissionObjective(e) => {
            let label = e.text.as_deref().unwrap_or(e.objective_id.as_str());
            match &e.state {
                Some(state) => {
                    let state = match state {
                        starstats_core::MissionObjectiveState::InProgress => "in progress",
                        starstats_core::MissionObjectiveState::Completed => "completed",
                        starstats_core::MissionObjectiveState::Failed => "failed",
                        starstats_core::MissionObjectiveState::Withdrawn => "withdrawn",
                        starstats_core::MissionObjectiveState::Unknown => "unknown",
                    };
                    format!("Objective: {label} ({state})")
                }
                None => format!("Objective: {label}"),
            }
        }
        GameEvent::QuantumRoute(e) => format!(
            "Route plotted: {} → {} ({})",
            e.start_system, e.destination, e.vehicle_class
        ),
        // No destination in the source line, so none is claimed here.
        GameEvent::QuantumArrived(e) => {
            format!("Quantum travel complete ({})", e.vehicle_class)
        }
        GameEvent::ItemEquipChange(e) => match e.action {
            starstats_core::EquipAction::Equip => match &e.port {
                Some(port) => format!("Equipped {} ({port})", e.item_class),
                None => format!("Equipped {}", e.item_class),
            },
            starstats_core::EquipAction::Store => format!("Stored {}", e.item_class),
        },
    }
}

// -- Device pairing --------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PairOutcome {
    pub claimed_handle: String,
    pub label: String,
}

#[derive(Debug, Deserialize)]
struct RedeemResponseBody {
    token: String,
    label: String,
    /// Server-assigned UUID for this device pairing. Surfaced for
    /// future self-revoke + diagnostic logging — the tray captures
    /// it now (rather than ignoring the field) so we have it on
    /// disk if a later slice adds an "unpair this device" button.
    /// `#[allow(dead_code)]` until that slice lands; matches the
    /// pattern used for `RequireAdmin.0` in starstats-server.
    #[allow(dead_code)]
    device_id: uuid::Uuid,
}

/// Redeem an 8-character pairing code against the API and persist
/// the returned device JWT into the local config. Once this returns
/// success, the sync worker can drain queued events without further
/// user action.
///
/// The user's `claimed_handle` is decoded from the token's
/// `preferred_username` so it stays in sync with whatever the API
/// believes it should be — important if a future migration renames
/// handles.
#[tauri::command(rename_all = "snake_case")]
pub async fn pair_device(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    api_url: String,
    code: String,
) -> Result<PairOutcome, String> {
    let api_url = api_url.trim_end_matches('/').to_string();
    validate_pair_url(&api_url)?;
    let url = format!("{api_url}/v1/auth/devices/redeem");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "code": code.trim().to_uppercase() }))
        .send()
        .await
        .map_err(|e| format!("contact api: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("server returned {status}: {body}"));
    }

    let parsed: RedeemResponseBody = resp
        .json()
        .await
        .map_err(|e| format!("parse response: {e}"))?;

    let claimed_handle = decode_username_from_token(&parsed.token)
        .ok_or_else(|| "token did not contain preferred_username".to_string())?;

    // M-T6: the device JWT lives in the OS keychain, not config.toml. Write it
    // there FIRST — `config::save` no longer persists the token, so the
    // keychain is the only durable home. Set the in-memory field too so the
    // respawn below (and any same-tick reader) sees it without a reload.
    crate::secret::SecretStore::new(crate::secret::ACCOUNT_DEVICE_TOKEN)
        .and_then(|store| store.set(&parsed.token))
        .map_err(|e| e.to_string())?;

    // Persist the rest into the local config — keeps the sync worker happy
    // and means the user doesn't have to re-enter anything.
    let mut cfg = config::load().map_err(|e| e.to_string())?;
    cfg.remote_sync.api_url = Some(api_url.clone());
    cfg.remote_sync.access_token = Some(parsed.token.clone());
    cfg.remote_sync.claimed_handle = Some(claimed_handle.clone());
    cfg.remote_sync.enabled = true;
    config::save(&cfg).map_err(|e| e.to_string())?;

    // Notify the UI so App's canonical config reflects the new pairing. App
    // only loads config at mount + on this event, and the Settings pane
    // re-derives its draft from App's config on remount — without this, the
    // paired state vanished when the user switched tabs (claimed_handle is
    // what drives the paired display). The device token is #[serde(skip)] so
    // it stays out of this payload — keychain-only (M-T6).
    {
        use tauri::Emitter;
        if let Err(e) = app_handle.emit("config-changed", &cfg) {
            tracing::warn!(error = %e, "pair_device: emit config-changed failed");
        }
    }

    // Reset auth_lost — we just minted a fresh token. Order matters:
    // clear auth_lost BEFORE respawn so the new worker doesn't read
    // a stale `auth_lost = true` from the previous session and skip
    // its first drain.
    {
        let mut s = state.account_status.lock();
        s.auth_lost = false;
        s.email_verified = None;
    }

    // Respawn the sync worker with the just-persisted token. Previously
    // this required a tray restart — the worker spawned at boot with
    // `enabled = false` returned None and there was no mechanism to
    // start a fresh one. Mirrors the save_config respawn pattern.
    sync::respawn(
        Arc::clone(&state.storage),
        Arc::clone(&state.sync_stats),
        Arc::clone(&state.account_status),
        Arc::clone(&state.sync_kick),
        Arc::clone(&state.sync_handle),
        app_handle,
        Arc::clone(&state.location_catalog),
    );

    // Best-effort: hydrate email_verified for the UI banner. If the
    // call fails (network blip), the banner just stays absent until
    // the next refresh — not worth failing the pair for.
    if let Ok(Some(me)) = sync::fetch_me(&api_url, &parsed.token).await {
        let mut s = state.account_status.lock();
        s.email_verified = Some(me.email_verified);
    }

    Ok(PairOutcome {
        claimed_handle,
        label: parsed.label,
    })
}

/// Reject pairing URLs that would leak the pairing code to a hostile
/// scheme. We allow `https://...` for production and `http://localhost`
/// (or `http://127.0.0.1`) for local development; everything else —
/// `javascript:`, `file:`, plain `http://example.com`, etc. — is
/// refused before the POST goes out.
fn validate_pair_url(api_url: &str) -> Result<(), String> {
    if let Some(rest) = api_url.strip_prefix("https://") {
        if rest.is_empty() {
            return Err("API URL must include a host".to_string());
        }
        return Ok(());
    }
    if let Some(rest) = api_url.strip_prefix("http://") {
        let host = rest.split('/').next().unwrap_or("");
        let host_only = host.split(':').next().unwrap_or("");
        if host_only == "localhost" || host_only == "127.0.0.1" {
            return Ok(());
        }
        return Err("API URL must be https:// (http:// is only allowed for localhost)".to_string());
    }
    Err("API URL must start with https:// (or http://localhost for dev)".to_string())
}

/// Manually nudge the sync worker so the next batch ships without
/// waiting for the configured interval. No-op (still returns Ok) if
/// the worker isn't running — the user gets the same UX whether it
/// fires immediately or sits idle because remote sync is disabled.
#[tauri::command(rename_all = "snake_case")]
pub fn retry_sync_now(state: State<'_, AppState>) -> Result<(), String> {
    // Wake BOTH lanes — priority and bulk. A single notify_one() woke only
    // one, so "Sync now" left the other lane snoozing until its interval.
    state.sync_kick.kick_all();
    Ok(())
}

/// Snapshot of the upload queue, for the tray's backlog readout.
///
/// Distinct from [`SyncStats`], which is lifetime-of-process counters
/// from the worker's point of view. This is the DB's point of view:
/// how much is still on this machine and how long clearing it will
/// plausibly take.
#[derive(Debug, Clone, Serialize)]
pub struct SyncBacklog {
    /// Rows still waiting to upload (`sent_at IS NULL`).
    pub pending: i64,
    /// Rows the poison-pill path shelved. Not included in `pending` —
    /// they only re-enter the queue via `release_quarantined`.
    pub quarantined: i64,
    /// Whether Star Citizen is currently running. The drain runs either
    /// way; this only explains why a large backlog is draining at the
    /// paced rate rather than the burst rate.
    pub game_running: bool,
    /// True when the queue is deep enough that the worker will be using
    /// its catch-up page size and cadence.
    pub catching_up: bool,
    /// Page size the next drain will actually use.
    pub effective_batch_size: usize,
    /// Rough seconds to clear `pending` at the current cadence. `None`
    /// when the queue is empty or remote sync is off.
    pub eta_secs: Option<u64>,
}

/// Assumed wall-clock cost of one `/v1/ingest` round-trip (request
/// build + upload + server insert + response), used only for the
/// user-facing ETA. Deliberately pessimistic so the estimate reads as
/// "no worse than" rather than as a promise.
const ASSUMED_BATCH_ROUND_TRIP: Duration = Duration::from_millis(1500);

/// Rough seconds to drain `pending` events at `page` events per cycle,
/// where each cycle costs one round-trip plus `delay`.
///
/// Pure so the arithmetic is testable without a worker, a DB, or a
/// network. Returns `None` for an empty queue (nothing to estimate) or
/// a zero page size (would divide by zero).
fn estimate_drain_secs(pending: i64, page: usize, delay: Duration) -> Option<u64> {
    if pending <= 0 || page == 0 {
        return None;
    }
    let cycles = (pending as u128).div_ceil(page as u128);
    let per_cycle = ASSUMED_BATCH_ROUND_TRIP.saturating_add(delay).as_millis();
    Some(
        cycles
            .saturating_mul(per_cycle)
            .div_ceil(1000)
            .min(u64::MAX as u128) as u64,
    )
}

/// Read the upload queue depth plus the cadence it will drain at.
///
/// Polled by the tray's sync card. Two `COUNT(*)`s against the partial
/// `sent_at` indexes — cheap enough for the existing status tick even
/// on a six-figure backlog, but NOT called from the drain hot path
/// (which infers backlog from a full page instead).
///
/// Sizing decisions come from [`sync::DrainTuning`], the same type the
/// worker uses, so the readout can never drift from the behaviour it
/// describes.
#[tauri::command(rename_all = "snake_case")]
pub fn get_sync_backlog(state: State<'_, AppState>) -> Result<SyncBacklog, String> {
    let pending = state.storage.count_unsent().map_err(|e| e.to_string())?;
    let quarantined = state
        .storage
        .count_quarantined()
        .map_err(|e| e.to_string())?;

    // Read the live config rather than a captured copy: the user may
    // have changed the cadence since the worker spawned (a respawn is
    // already queued in that case).
    let cfg = config::load().map_err(|e| e.to_string())?;
    let tuning = sync::DrainTuning::from_config(&cfg.remote_sync);

    // "Catching up" is defined the way the worker defines it: the next
    // page would come back full.
    let catching_up = pending > tuning.steady_page() as i64;
    let game_running = if catching_up {
        crate::process_guard::is_starcitizen_running()
    } else {
        false
    };

    let effective_batch_size = tuning.page_size(catching_up, game_running);
    let delay = tuning.delay(
        catching_up,
        game_running,
        Duration::from_secs(cfg.remote_sync.interval_secs.max(5)),
        0,
    );

    let eta_secs = if cfg.remote_sync.enabled {
        estimate_drain_secs(pending, effective_batch_size, delay)
    } else {
        None
    };

    Ok(SyncBacklog {
        pending,
        quarantined,
        game_running,
        catching_up,
        effective_batch_size,
        eta_secs,
    })
}

#[cfg(test)]
mod sync_backlog_tests {
    use super::*;

    #[test]
    fn an_empty_queue_has_no_eta() {
        assert_eq!(estimate_drain_secs(0, 200, Duration::from_secs(60)), None);
        assert_eq!(estimate_drain_secs(-1, 200, Duration::from_secs(60)), None);
    }

    #[test]
    fn a_zero_page_size_does_not_divide_by_zero() {
        assert_eq!(estimate_drain_secs(1000, 0, Duration::from_secs(60)), None);
    }

    #[test]
    fn the_old_cadence_is_what_made_a_backlog_hopeless() {
        // 300k events, 200 per batch, one batch per 60 s: 1500 cycles at
        // ~61.5 s each — over 25 hours. This is the number the user hit.
        let old = estimate_drain_secs(300_000, 200, Duration::from_secs(60)).unwrap();
        assert!(
            old > 24 * 3600,
            "old cadence should be a day-plus, got {old}s"
        );
    }

    #[test]
    fn catch_up_cadence_clears_the_same_backlog_in_minutes() {
        // Same 300k events at the catch-up page + delay: 150 cycles at
        // ~1.75 s each.
        let new = estimate_drain_secs(300_000, 2000, Duration::from_millis(250)).unwrap();
        assert!(
            new < 15 * 60,
            "catch-up should clear 300k in under 15 min, got {new}s"
        );
    }

    #[test]
    fn a_partial_final_batch_still_counts_as_a_cycle() {
        // Anything from 1..=200 events fits one cycle...
        let zero_delay = Duration::from_secs(0);
        let single = estimate_drain_secs(1, 200, zero_delay).unwrap();
        assert_eq!(estimate_drain_secs(200, 200, zero_delay).unwrap(), single);
        // ...and 201 spills into a second one, so the estimate must rise.
        // (Asserting the ratio would be wrong: the seconds rounding is
        // applied once to the total, not per cycle.)
        assert!(estimate_drain_secs(201, 200, zero_delay).unwrap() > single);
    }
}

/// One event type's local-vs-remote comparison.
#[derive(Debug, Clone, Serialize)]
pub struct DriftRow {
    pub event_type: String,
    /// Rows this client believes it delivered (`sent_at` is a real stamp).
    pub local_sent: u64,
    /// Rows the server reports holding, from its rollup.
    pub remote: u64,
    /// `local_sent - remote`. DIAGNOSTIC ONLY — do not sum these to decide
    /// whether anything is missing. A positive value usually means the local
    /// classifier renamed this type after the events were uploaded, not that
    /// the server lost them; the matching negative appears under the old
    /// name. See the note in `check_upload_drift`.
    pub missing: i64,
}

/// Result of an on-demand drift check.
#[derive(Debug, Clone, Serialize)]
pub struct UploadDrift {
    pub checked_at: String,
    pub local_sent_total: u64,
    pub remote_total: u64,
    /// How many events the server is short overall, from TOTALS. Zero
    /// whenever the server holds at least as much as this device sent —
    /// which is the only honest basis for offering a re-upload.
    pub shortfall_total: u64,
    /// How many MORE the server holds than this device ever sent. Normal:
    /// other devices, or history predating this local database.
    pub surplus_total: u64,
    /// Rows still queued. Shown for context so a queue mid-drain is not
    /// mistaken for drift.
    pub pending: i64,
    /// Types where the two sides disagree, largest gap first. Types that
    /// agree are omitted — on a healthy client this list is empty.
    pub rows: Vec<DriftRow>,
}

/// Compare local delivered-event counts against the server's, per type.
///
/// Exists because nothing else can notice this. The client marks a row
/// `sent_at` on a 2xx and never revisits it, so if the SERVER later loses
/// data — restore from an older backup, a wiped environment — the client
/// still believes it delivered everything and the upload queue reads zero
/// forever. The events are sitting in local SQLite, unreachable.
///
/// Deliberately on-demand. Nothing calls this on a timer: drift changes on
/// the timescale of server incidents, and putting it on the status tick
/// would turn a rare diagnostic into steady background load for both sides.
///
/// Cost: one `GET /v1/me/summary`, which the server answers from its
/// `stat_event_counts` rollup, plus one grouped local query over an indexed
/// column. The comparison itself is done here on the client.
#[tauri::command(rename_all = "snake_case")]
pub async fn check_upload_drift(state: State<'_, AppState>) -> Result<UploadDrift, String> {
    let cfg = config::load().map_err(|e| e.to_string())?;
    let api_url = cfg
        .remote_sync
        .api_url
        .clone()
        .filter(|u| !u.is_empty())
        .ok_or_else(|| "No API URL configured".to_string())?;
    let token = cfg
        .remote_sync
        .access_token
        .clone()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "This device is not paired — pair it before checking".to_string())?;

    // Local half first, so a server hiccup does not cost us the cheap part.
    let local: Vec<(String, u64)> = state
        .storage
        .sent_counts_by_type()
        .map_err(|e| e.to_string())?;
    let pending = state.storage.count_unsent().map_err(|e| e.to_string())?;

    let remote = sync::fetch_summary(&api_url, &token)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "The server rejected this device's token — re-pair and try again".to_string()
        })?;

    let remote_by_type: std::collections::HashMap<String, u64> = remote
        .by_type
        .into_iter()
        .map(|t| (t.event_type, t.count))
        .collect();

    let local_sent_total: u64 = local.iter().map(|(_, n)| *n).sum();

    let mut rows: Vec<DriftRow> = Vec::new();
    for (event_type, local_sent) in local {
        let remote_count = remote_by_type.get(&event_type).copied().unwrap_or(0);
        let missing = local_sent as i64 - remote_count as i64;
        if missing != 0 {
            rows.push(DriftRow {
                event_type,
                local_sent,
                remote: remote_count,
                missing,
            });
        }
    }
    rows.sort_by(|a, b| b.missing.cmp(&a.missing));

    // The VERDICT comes from totals, never from summed per-type gaps.
    //
    // Per-type counts disagree for a reason that has nothing to do with loss:
    // `reparse_events` rewrites the local `type` column in place, while the
    // server keeps whatever name was current when the event was uploaded. The
    // idempotency key is derived from (log_source, file_sig, offset, line) and
    // does NOT include the type, so a re-upload hits ON CONFLICT DO NOTHING
    // and the server's name never changes. Renamed types therefore show as a
    // local surplus on the new name and a server surplus on the old one, for
    // ever, and resending provably cannot close the gap.
    //
    // Summing the positive gaps counted exactly that mismatch as "missing".
    // On a real machine that read as 273k events needing re-upload while the
    // server actually held 5,239 MORE than the client had ever sent. Offering
    // a resend button there is worse than useless: it is a false claim
    // attached to an action that cannot help.
    //
    // Totals cannot produce that false positive. They can in principle hide a
    // genuine loss that is exactly offset by extra server-side rows, which is
    // both rarer and far less harmful than telling someone to re-upload a
    // third of a million events for nothing.
    let shortfall_total = local_sent_total.saturating_sub(remote.total);
    let surplus_total = remote.total.saturating_sub(local_sent_total);

    Ok(UploadDrift {
        checked_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        local_sent_total,
        remote_total: remote.total,
        shortfall_total,
        surplus_total,
        pending,
        rows,
    })
}

/// Put delivered rows of the named types back in the upload queue, then wake
/// the sync worker.
///
/// Takes explicit types rather than "re-upload everything" so recovery is
/// scoped to what the drift check actually found missing. Re-sending is safe
/// regardless — `/v1/ingest` dedupes on `idempotency_key` — but resending
/// hundreds of thousands of events the server already holds is pointless
/// traffic for both sides.
///
/// Returns the number of rows re-queued.
#[tauri::command(rename_all = "snake_case")]
pub fn requeue_missing_events(
    state: State<'_, AppState>,
    event_types: Vec<String>,
) -> Result<u64, String> {
    let refs: Vec<&str> = event_types.iter().map(|s| s.as_str()).collect();
    let n = state
        .storage
        .requeue_sent_for_types(&refs)
        .map_err(|e| e.to_string())?;
    if n > 0 {
        // Wake both lanes so the re-queued rows start moving immediately
        // rather than waiting out the current interval.
        state.sync_kick.kick_all();
    }
    Ok(n)
}

/// Count rows the sync worker has quarantined (rows whose `sent_at`
/// starts with `__quarantined_`). Read by the SettingsPane Recovery
/// affordance to decide whether to surface the "Release N" button.
/// `i64` mirrors the storage method; values are always >= 0.
#[tauri::command(rename_all = "snake_case")]
pub fn count_quarantined(state: State<'_, AppState>) -> Result<i64, String> {
    state.storage.count_quarantined().map_err(|e| e.to_string())
}

/// Release every row the sync worker's poison-pill path has
/// quarantined. Flips `sent_at` back to NULL on every
/// `__quarantined_*` row, then kicks the sync worker so the next
/// drain re-attempts them. Returns the count released for UI display.
///
/// Recovery affordance for the case where mass-quarantine has
/// stockpiled rows that should have been retried (typically a
/// transient batch-level 4xx that bisection mis-attributed to each
/// event). After release the cap (`MAX_QUARANTINES_PER_DRAIN`) keeps
/// any persistent failure from runaway-re-quarantining all in one
/// drain — the user can keep pressing the button (or fix the
/// underlying cause) until the queue clears.
#[tauri::command(rename_all = "snake_case")]
pub fn release_quarantined(state: State<'_, AppState>) -> Result<u64, String> {
    let n = state
        .storage
        .release_quarantined()
        .map_err(|e| e.to_string())?;
    if n > 0 {
        state.sync_kick.kick_all();
    }
    Ok(n)
}

/// Wake the hangar refresh worker immediately instead of waiting for
/// its next REFRESH_INTERVAL tick. Wired up by the Status pane's
/// "Refresh now" button. Silent no-op if the worker isn't spawned
/// (i.e. user hasn't paired their device yet) — the Notify just
/// queues a permit nobody consumes, costs nothing.
///
/// The cycle still respects per-tick gates (game running, no cookie
/// set, auth_lost) — kicking doesn't bypass safety, only the sleep.
#[tauri::command(rename_all = "snake_case")]
pub fn refresh_hangar_now(state: State<'_, AppState>) -> Result<(), String> {
    state.hangar_kick.notify_one();
    Ok(())
}

/// Pull `preferred_username` out of a JWT's payload without verifying
/// the signature — the server already verified it for us when it
/// minted the token. This is purely a UX convenience so we can show
/// the right handle on the next render.
fn decode_username_from_token(token: &str) -> Option<String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("preferred_username")?.as_str().map(str::to_string)
}

// -- RSI session cookie management ----------------------------------
//
// The hangar fetcher (see `crate::hangar`) needs an authenticated
// RSI session cookie to read the user's pledge ledger. The user
// pastes that cookie value out of their browser DevTools; the tray
// stores it in the OS keychain via `SecretStore` and never displays
// it back. The three commands below — `set` / `clear` /
// `get_status` — are deliberately read-only with respect to the
// secret value itself: only a redacted preview ever leaves the host.

/// Upper bound on the cookie value length. The real RSI session
/// cookie is ~50–100 chars; 4096 is paranoid headroom that still
/// rejects accidental whole-page paste.
pub const MAX_COOKIE_CHARS: usize = 4096;

#[derive(Debug, Clone, Serialize)]
pub struct RsiCookieStatus {
    pub configured: bool,
    /// Last-4-character preview prefixed with an ellipsis (e.g.
    /// "…ab12"). Lets the user confirm "yes, I'm set up" without
    /// re-displaying the secret. `None` when no cookie is stored.
    pub preview: Option<String>,
}

/// Persist the user's pasted RSI session cookie value into the OS
/// keychain. Idempotent — overwrites any previous value. Returns the
/// redacted preview so the UI can confirm the write without echoing
/// the secret.
///
/// Takes `cookie_value` as a top-level Tauri command arg. The
/// command carries `#[tauri::command(rename_all = "snake_case")]`,
/// so the IPC key is the verbatim param name — JS must invoke with
/// `{ cookie_value }`. NOTE: there is NO automatic camelCase→snake_case
/// mapping; under a bare `#[tauri::command]` (tauri-macros ≥2.6) the
/// key would default to camelCase (`cookieValue`) and a snake_case
/// payload would silently bind to `None`. The `rename_all` attribute
/// is what makes the byte-exact-snake_case invariant true (C1 fix,
/// 2026-07-09). The earlier `SetRsiCookieRequest { cookie_value }`
/// wrapper struct was rejected at runtime with
/// `missing field 'cookie_value'`; a flat arg avoids that.
#[tauri::command(rename_all = "snake_case")]
pub async fn set_rsi_cookie(cookie_value: String) -> Result<RsiCookieStatus, String> {
    let trimmed = cookie_value.trim();
    if trimmed.is_empty() {
        return Err("cookie value is empty".into());
    }
    if trimmed.chars().count() > MAX_COOKIE_CHARS {
        return Err("cookie value too long".into());
    }
    let store = SecretStore::new(ACCOUNT_RSI_SESSION_COOKIE).map_err(|e| e.to_string())?;
    store.set(trimmed).map_err(|e| e.to_string())?;
    Ok(RsiCookieStatus {
        configured: true,
        preview: Some(redact(trimmed)),
    })
}

/// Remove the stored cookie from the keychain. Idempotent — clearing
/// a missing entry is a no-op so the UI's "Forget cookie" path can
/// call this unconditionally.
#[tauri::command(rename_all = "snake_case")]
pub async fn clear_rsi_cookie() -> Result<RsiCookieStatus, String> {
    let store = SecretStore::new(ACCOUNT_RSI_SESSION_COOKIE).map_err(|e| e.to_string())?;
    store.clear().map_err(|e| e.to_string())?;
    Ok(RsiCookieStatus {
        configured: false,
        preview: None,
    })
}

/// Probe the keychain for the current RSI cookie status. Read-only —
/// returns just the redacted preview, never the secret.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_rsi_cookie_status() -> Result<RsiCookieStatus, String> {
    let store = SecretStore::new(ACCOUNT_RSI_SESSION_COOKIE).map_err(|e| e.to_string())?;
    let stored = store.get().map_err(|e| e.to_string())?;
    let preview = stored.as_deref().map(redact);
    Ok(RsiCookieStatus {
        configured: stored.is_some(),
        preview,
    })
}

/// Build a redacted preview ("…XYZA") of a cookie value. Last four
/// characters are kept so the user can disambiguate two pastes from
/// the same browser without exposing meaningful prefix entropy.
fn redact(s: &str) -> String {
    let last4: String = s
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{last4}")
}

// -- Org-connector bearer token management --------------------------
//
// The opt-in org-platform connector authenticates with a bearer token
// the user enters. Like the RSI cookie (M-T6), it lives in the OS
// keychain and is never displayed back — these commands are write-only
// w.r.t. the secret; only a redacted preview leaves the host. Setting
// or clearing it respawns the connector so the change takes effect
// immediately (the connector reads the token once, at spawn).

/// Upper bound on the bearer length — same paranoid headroom as the cookie.
pub const MAX_BEARER_CHARS: usize = 4096;

#[derive(Debug, Clone, Serialize)]
pub struct OrgBearerStatus {
    pub configured: bool,
    /// Redacted "…WXYZ" preview so the user can confirm a value is stored
    /// without re-displaying the secret. `None` when nothing is stored.
    pub preview: Option<String>,
}

/// Persist the org-platform bearer token into the OS keychain and respawn
/// the connector so it reconnects with the new credential. Idempotent —
/// overwrites any previous value. Returns the redacted preview.
#[tauri::command(rename_all = "snake_case")]
pub async fn set_org_bearer(
    state: State<'_, AppState>,
    bearer_token: String,
) -> Result<OrgBearerStatus, String> {
    let trimmed = bearer_token.trim();
    if trimmed.is_empty() {
        return Err("bearer token is empty".into());
    }
    if trimmed.chars().count() > MAX_BEARER_CHARS {
        return Err("bearer token too long".into());
    }
    let store = SecretStore::new(ACCOUNT_ORG_BEARER).map_err(|e| e.to_string())?;
    store.set(trimmed).map_err(|e| e.to_string())?;
    respawn_org_connector(&state);
    Ok(OrgBearerStatus {
        configured: true,
        preview: Some(redact(trimmed)),
    })
}

/// Remove the stored bearer from the keychain and respawn (the connector
/// stops if it was enabled — no token, no link). Idempotent.
#[tauri::command(rename_all = "snake_case")]
pub async fn clear_org_bearer(state: State<'_, AppState>) -> Result<OrgBearerStatus, String> {
    let store = SecretStore::new(ACCOUNT_ORG_BEARER).map_err(|e| e.to_string())?;
    store.clear().map_err(|e| e.to_string())?;
    respawn_org_connector(&state);
    Ok(OrgBearerStatus {
        configured: false,
        preview: None,
    })
}

/// Probe the keychain for the current bearer status. Read-only — returns
/// just the redacted preview, never the secret.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_org_bearer_status() -> Result<OrgBearerStatus, String> {
    let store = SecretStore::new(ACCOUNT_ORG_BEARER).map_err(|e| e.to_string())?;
    let stored = store.get().map_err(|e| e.to_string())?;
    let preview = stored.as_deref().map(redact);
    Ok(OrgBearerStatus {
        configured: stored.is_some(),
        preview,
    })
}

/// Respawn the org connector so a bearer set/clear takes effect without an
/// app restart. Mirrors the respawn call in `save_config`; the connector
/// re-reads the (now keychain-hydrated) config on spawn.
fn respawn_org_connector(state: &AppState) {
    org_connector::respawn(
        Arc::clone(&state.storage),
        Arc::clone(&state.location_catalog),
        Arc::clone(&state.org_connector_handle),
        Arc::clone(&state.tail_event_kick),
    );
}

// === Health surface (added 2026-05-16) =================================

/// 60-second TTL cache for the two `sysinfo`-derived health inputs.
/// `get_health` is polled every 15s by the tray UI; constructing a
/// fresh `System` + `Disks` on each poll is individually cheap but
/// cumulatively wasteful when the tray idles for hours. The tuple is
/// `(stamped_at, sc_process_running, disk_free_bytes)`. We tolerate the
/// staleness — the SC-running and free-disk signals don't need
/// sub-minute resolution for the Health card to be useful.
static SYSINFO_CACHE: parking_lot::Mutex<Option<(std::time::Instant, bool, Option<u64>)>> =
    parking_lot::Mutex::new(None);

const SYSINFO_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Returns `(sc_process_running, disk_free_bytes)` from the cache if
/// the entry is younger than `SYSINFO_TTL`, otherwise recomputes,
/// stores, and returns the fresh values. Lock is released before the
/// expensive recompute so a contended call doesn't serialize behind
/// the cache holder.
fn cached_sysinfo() -> (bool, Option<u64>) {
    let now = std::time::Instant::now();
    {
        let cache = SYSINFO_CACHE.lock();
        if let Some((stamped, sc, free)) = *cache {
            if now.duration_since(stamped) < SYSINFO_TTL {
                return (sc, free);
            }
        }
    }
    let (sc, free) = compute_sysinfo();
    *SYSINFO_CACHE.lock() = Some((now, sc, free));
    (sc, free)
}

/// Uncached `sysinfo` read used by `cached_sysinfo`. Constructs a
/// minimal `System` (processes only, no global memory/CPU refresh) and
/// queries the partition that hosts the StarStats data directory.
fn compute_sysinfo() -> (bool, Option<u64>) {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    let sys =
        System::new_with_specifics(RefreshKind::new().with_processes(ProcessRefreshKind::new()));
    let sc_running = sys
        .processes_by_name("StarCitizen.exe".as_ref())
        .next()
        .is_some()
        || sys
            .processes_by_name("StarCitizen".as_ref())
            .next()
            .is_some();
    let free = crate::config::data_dir()
        .ok()
        .and_then(|d| free_bytes_for_path(&d));
    (sc_running, free)
}

/// Assemble a `HealthInputs` snapshot from `AppState`, `Config`, the
/// secret store, and `sysinfo`. Pure read-only — never mutates state.
fn snapshot_health_inputs(state: &AppState) -> Result<crate::health::HealthInputs, String> {
    let now = chrono::Utc::now();

    let tail = state.tail_stats.lock().clone();
    let sync_snap = state.sync_stats.lock().clone();
    let hangar = state.hangar_stats.lock().clone();
    let account = state.account_status.lock().clone();
    let update_avail = state.update_available.lock().clone();

    let config = crate::config::load().map_err(|e| e.to_string())?;
    let gamelog_override_set = config.gamelog_path.is_some();
    let discovered = crate::discovery::discover();

    let cookie_configured = SecretStore::new(ACCOUNT_RSI_SESSION_COOKIE)
        .ok()
        .and_then(|s| s.get().ok())
        .flatten()
        .is_some();

    let (sc_process_running, disk_free_bytes) = cached_sysinfo();

    // Parse RFC3339 timestamps into DateTime<Utc>. A malformed value
    // disables the dependent check (e.g. GameLogStale), so log on
    // failure rather than silently swallow — without the warn, a
    // regression in the upstream timestamp shape would mask the
    // staleness signal indefinitely.
    let parse_dt = |label: &str, s: &Option<String>| -> Option<chrono::DateTime<chrono::Utc>> {
        let raw = s.as_deref()?;
        match chrono::DateTime::parse_from_rfc3339(raw) {
            Ok(d) => Some(d.with_timezone(&chrono::Utc)),
            Err(e) => {
                tracing::warn!(
                    field = label,
                    raw = raw,
                    error = %e,
                    "health snapshot: dropping malformed RFC3339 timestamp"
                );
                None
            }
        }
    };
    let tail_last_event_at = parse_dt("tail.last_event_at", &tail.last_event_at);
    let hangar_last_attempt_at = parse_dt("hangar.last_attempt_at", &hangar.last_attempt_at);
    let hangar_last_success_at = parse_dt("hangar.last_success_at", &hangar.last_success_at);

    Ok(crate::health::HealthInputs {
        now,
        gamelog_discovered_count: discovered.len(),
        gamelog_override_set,
        remote_sync_enabled: config.remote_sync.enabled,
        api_url: config.remote_sync.api_url.clone(),
        access_token: config.remote_sync.access_token.clone(),
        web_origin: config.web_origin.clone(),
        auth_lost: account.auth_lost,
        email_verified: account.email_verified,
        cookie_configured,
        sync_last_error: sync_snap.last_error.clone(),
        // The SyncStats type tracks per-attempt counters elsewhere; for
        // now this is left at zero. A future commit can plumb the
        // attempts-since-success counter through.
        sync_attempts_since_success: 0,
        hangar_last_attempt_at,
        hangar_last_success_at,
        hangar_last_skip_reason: hangar.last_skip_reason.clone(),
        tail_current_path: tail
            .current_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        tail_last_event_at,
        sc_process_running,
        disk_free_bytes,
        update_available_version: update_avail.as_ref().map(|u| u.version.clone()),
        dismissed: config.dismissed_health.clone(),
    })
}

/// Best-effort free-space query for the partition containing `path`.
/// Returns `None` on platforms or paths where it fails.
fn free_bytes_for_path(path: &std::path::Path) -> Option<u64> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_health(
    state: State<'_, AppState>,
) -> Result<Vec<crate::health::HealthItem>, String> {
    let inputs = snapshot_health_inputs(&state)?;
    Ok(crate::health::current_health(&inputs))
}

/// Process-wide lock taken by `dismiss_health` (and any future
/// command that does load-mutate-save on `config.toml`) to prevent
/// the load+save race: two concurrent dismissals would otherwise
/// each load the pre-dismissal config, push their own item, then
/// the second write would clobber the first. The lock is module-
/// private; callers reach it only via the command surface.
static CONFIG_MUTATION_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[tauri::command(rename_all = "snake_case")]
pub async fn dismiss_health(
    id: crate::health::HealthId,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let inputs = snapshot_health_inputs(&state)?;
    let live = crate::health::current_health(&inputs);
    let target = live
        .iter()
        .find(|i| i.id == id)
        .ok_or_else(|| format!("No live HealthItem with id {:?}", id))?;
    if !target.dismissible {
        return Err(format!("HealthItem {:?} is not dismissible", id));
    }
    let _guard = CONFIG_MUTATION_LOCK.lock();
    let mut config = crate::config::load().map_err(|e| e.to_string())?;
    config
        .dismissed_health
        .push(crate::health::DismissedHealth {
            id: target.id,
            fingerprint: target.fingerprint.clone(),
            dismissed_at: chrono::Utc::now(),
        });
    crate::config::save(&config).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn check_api_url(url: String) -> Result<crate::probes::ApiUrlCheck, String> {
    Ok(crate::probes::check_api_url(url).await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn check_rsi_cookie(cookie: String) -> Result<crate::probes::CookieCheck, String> {
    Ok(crate::probes::check_rsi_cookie(cookie).await)
}

/// Set by `apps/tray-ui/src/updater.ts` after a successful auto-update
/// or manual update check that found a newer version. Feeds the
/// `HealthId::UpdateAvailable` item in the health surface.
///
/// Validates the version string: must look like semver-ish (digits,
/// dots, dashes, plus, ascii alphanumerics) and be at most 64 chars.
/// Renderer-controllable surface, so any compromise/bug in the JS
/// layer can't inject arbitrary text into the Health card via this
/// path.
#[tauri::command(rename_all = "snake_case")]
pub async fn set_update_available(
    version: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let version = version.trim();
    if version.is_empty() || version.len() > 64 {
        return Err("invalid version: must be 1-64 chars".into());
    }
    if !version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
    {
        return Err("invalid version: only alphanumerics, '.', '-', '+', '_' allowed".into());
    }
    *state.update_available.lock() = Some(crate::state::UpdateInfo {
        version: version.to_string(),
        checked_at: chrono::Utc::now(),
    });
    Ok(())
}

/// Default interest cutoff for the review pane and the badge count.
/// Matches the spec: only shapes scoring >= 50 surface to the user.
/// Lower scores stay cached for diagnostics but don't get promoted.
const UNKNOWN_LINE_REVIEW_THRESHOLD: u8 = 50;

/// Return the persisted per-install anonymous ID for parser submissions,
/// generating one on first call. Format: `anon_<uuid v4 simple>` (the
/// `anon_` prefix keeps the value visually distinguishable from device
/// IDs, batch IDs, etc. in logs without leaking install identity).
///
/// The server requires `client_anon_id` to be non-empty; this helper is
/// the only producer client-side, so empty values can never reach the
/// wire. Safe to call repeatedly — the second call onwards is a config
/// read.
fn get_or_create_client_anon_id() -> anyhow::Result<String> {
    let mut cfg = config::load()?;
    let (id, dirty) = resolve_client_anon_id(cfg.client_anon_id.as_deref());
    if dirty {
        cfg.client_anon_id = Some(id.clone());
        config::save(&cfg)?;
    }
    Ok(id)
}

/// Pure helper carved out for testability: given the persisted value
/// (if any), return `(id, dirty)` where `dirty == true` means the
/// caller should write the new id back to disk. Empty / whitespace
/// values are treated as missing so a corrupted config still self-heals.
fn resolve_client_anon_id(existing: Option<&str>) -> (String, bool) {
    if let Some(existing) = existing {
        if !existing.trim().is_empty() {
            return (existing.to_string(), false);
        }
    }
    (format!("anon_{}", uuid::Uuid::new_v4().simple()), true)
}

/// Tauri command exposing the stable per-install anon ID to the UI.
/// Today only used for diagnostics — the submission path generates and
/// injects the value server-side so the frontend can't impersonate
/// another install.
#[tauri::command(rename_all = "snake_case")]
pub fn client_anon_id() -> Result<String, String> {
    get_or_create_client_anon_id().map_err(|e| e.to_string())
}

/// List every non-dismissed unknown shape worth reviewing (score >=
/// `UNKNOWN_LINE_REVIEW_THRESHOLD`). Ordered by the storage layer:
/// interest desc, occurrence desc, last_seen desc.
#[tauri::command(rename_all = "snake_case")]
pub fn list_unknown_lines(
    state: State<'_, AppState>,
) -> Result<Vec<starstats_core::UnknownLine>, String> {
    state
        .storage
        .list_unknown_lines(UNKNOWN_LINE_REVIEW_THRESHOLD)
        .map_err(|e| e.to_string())
}

/// Cheap counter for the tray badge. Returns how many shapes are
/// currently above the review threshold and not dismissed.
#[tauri::command(rename_all = "snake_case")]
pub fn count_unknown_lines(state: State<'_, AppState>) -> Result<u32, String> {
    state
        .storage
        .count_unknown_lines(UNKNOWN_LINE_REVIEW_THRESHOLD)
        .map_err(|e| e.to_string())
}

/// Hide a shape from the review pane. The row stays in SQLite so a
/// future re-capture of the same shape doesn't re-promote it — the
/// user told us once they don't care.
#[tauri::command(rename_all = "snake_case")]
pub fn dismiss_unknown_line(state: State<'_, AppState>, shape_hash: String) -> Result<(), String> {
    state
        .storage
        .dismiss_unknown_line(&shape_hash)
        .map_err(|e| e.to_string())
}

/// Ship a batch of user-reviewed shapes to `POST /v1/parser-submissions`
/// on the configured server. On success, stamps `submitted_at` on each
/// shape row locally so the review pane stops surfacing them.
///
/// HTTP shape mirrors `sync::drain_once`: 30s timeout, bearer auth
/// against the persisted device token, 401/403 flips `auth_lost` and
/// bails. The cursor pattern doesn't apply — submissions are one-shot
/// from a user action, not a continuous drain.
#[tauri::command(rename_all = "snake_case")]
pub async fn submit_unknown_lines(
    state: State<'_, AppState>,
    payloads: Vec<starstats_core::ParserSubmission>,
) -> Result<starstats_core::wire::ParserSubmissionResponse, String> {
    if payloads.is_empty() {
        return Ok(starstats_core::wire::ParserSubmissionResponse {
            accepted: 0,
            deduped: 0,
            ids: Vec::new(),
        });
    }

    // Stamp the anon ID server-side rather than trusting whatever the
    // frontend put in `client_anon_id`. This both fixes the bug where
    // the UI was sending `""` (which the server rejects with 400) and
    // closes the impersonation gap where one install could submit
    // under another install's ID by editing the JS payload.
    let anon_id = get_or_create_client_anon_id().map_err(|e| e.to_string())?;
    let payloads: Vec<starstats_core::ParserSubmission> = payloads
        .into_iter()
        .map(|p| starstats_core::ParserSubmission {
            client_anon_id: anon_id.clone(),
            ..p
        })
        .collect();

    let cfg = config::load().map_err(|e| e.to_string())?;
    let api_url = cfg
        .remote_sync
        .api_url
        .clone()
        .ok_or_else(|| "remote sync not configured: api_url missing".to_string())?;
    let access_token = cfg
        .remote_sync
        .access_token
        .clone()
        .ok_or_else(|| "remote sync not configured: device not paired".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    let url = format!("{}/v1/parser-submissions", api_url.trim_end_matches('/'));
    let batch = starstats_core::wire::ParserSubmissionBatch {
        submissions: payloads.clone(),
    };
    let resp = client
        .post(&url)
        .bearer_auth(&access_token)
        .json(&batch)
        .send()
        .await
        .map_err(|e| format!("POST /v1/parser-submissions: {e}"))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        // Mirror sync::drain_once — surface auth_lost so the existing
        // health banner picks it up. Submissions aren't critical
        // enough to clear the persisted token here; the next sync
        // drain will hit the same status and run that path.
        state.account_status.lock().auth_lost = true;
        return Err(format!("auth lost: parser-submissions returned {status}"));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("submissions failed: {status} {body}"));
    }

    let parsed: starstats_core::wire::ParserSubmissionResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse submissions response: {e}"))?;

    // Best-effort: stamp submitted_at locally so the review pane
    // hides the shapes the server accepted. We don't have per-row
    // accept/dedupe granularity in the response payload (the server
    // returns aggregate counts + ids), so we stamp every shape we
    // sent — the server already deduped server-side.
    let now = chrono::Utc::now().to_rfc3339();
    for p in &payloads {
        if let Err(e) = state.storage.mark_submitted(&p.shape_hash, &now) {
            tracing::warn!(
                shape_hash = %p.shape_hash,
                error = %e,
                "mark_submitted failed after successful POST",
            );
        }
    }

    Ok(parsed)
}

/// Current OS-level autostart state — reads the registry / autostart
/// entry rather than the persisted preference so the UI surfaces the
/// ground truth (catches the case where the entry was removed
/// externally, e.g. via Task Manager's Startup tab on Windows).
/// Wire shape returned to the tray frontend by
/// `get_reference_category`. Mirrors the public
/// `/v1/reference/{category}` listing response from
/// `starstats-server`; kept intentionally loose (entries are
/// `serde_json::Value`) because the frontend owns the typed shape
/// in `apps/tray-ui/src/lib/reference.ts` and the server-side
/// schema is allowed to grow without breaking the client.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ReferenceListingResponse {
    #[serde(default)]
    pub entries: Vec<serde_json::Value>,
}

/// HTTP relay for `GET {api_url}/v1/reference/{category}`.
///
/// The tray's WebView CSP restricts cross-origin `fetch()` from the
/// frontend (see `tauri.conf.json`), so the reference catalogue —
/// which lives at the paired StarStats server — has to be fetched
/// Rust-side and handed across the IPC bridge. Without this, the
/// pretty-name lookup that turns `AEGS_Avenger_Stalker` into
/// `Aegis Avenger Stalker` stays empty and event summaries render
/// with raw class identifiers.
///
/// Errors stringify on the way out (per `commands.rs` convention).
/// The frontend treats a rejection as "catalogue unavailable" and
/// falls back to the raw string — same degradation as a server-side
/// outage.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_reference_category(
    api_url: String,
    category: String,
) -> Result<ReferenceListingResponse, String> {
    let api_url = api_url.trim().trim_end_matches('/');
    if api_url.is_empty() {
        return Ok(ReferenceListingResponse::default());
    }
    // Scheme-validate the renderer-supplied base URL (https://, or
    // http:// only for localhost) so a compromised/malformed IPC call
    // can't point the Rust-side fetch at `file://` or an arbitrary host
    // (L9). Same allowlist the pairing flow enforces.
    validate_pair_url(api_url)?;
    // Validate category against the closed set the server exposes,
    // so a malicious / malformed IPC call can't synthesise an
    // arbitrary path under `/v1/reference/`.
    if !matches!(
        category.as_str(),
        "vehicle" | "weapon" | "item" | "location"
    ) {
        return Err(format!("unknown reference category: {category}"));
    }
    let url = format!("{api_url}/v1/reference/{category}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("contact api: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("server returned {}", resp.status()));
    }
    resp.json::<ReferenceListingResponse>()
        .await
        .map_err(|e| format!("parse response: {e}"))
}

/// HTTP relay for `GET {api_url}/v1/me/roadmap/whats-new`.
///
/// Same reasoning as `get_reference_category` — the tray's WebView CSP
/// blocks cross-origin renderer-side fetch, so the panel's payload
/// must come through the IPC bridge. The Rust client builds the
/// reqwest::Client per call (`from_config`) so a re-pair / unpair
/// reflects immediately. Errors stringify on the way out per the
/// `commands.rs` convention.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_whats_new() -> Result<crate::whats_new::WhatsNewResponse, String> {
    let cfg = config::load().map_err(|e| e.to_string())?;
    let client = crate::whats_new::WhatsNewClient::from_config(&cfg).map_err(|e| e.to_string())?;
    client.fetch_whats_new().await.map_err(|e| e.to_string())
}

/// HTTP relay for `POST {api_url}/v1/me/roadmap/whats-new/seen`. The
/// renderer passes the `(item_id, entry_id)` pair as the strings it
/// got off the panel response; we parse them back into UUIDs here so
/// a malformed string fails loudly at the IPC boundary rather than
/// silently no-op'ing the read-state row.
#[tauri::command(rename_all = "snake_case")]
pub async fn mark_whats_new_seen(item_id: String, entry_id: String) -> Result<(), String> {
    let item_id = uuid::Uuid::parse_str(&item_id).map_err(|e| format!("bad item_id: {e}"))?;
    let entry_id = uuid::Uuid::parse_str(&entry_id).map_err(|e| format!("bad entry_id: {e}"))?;
    let cfg = config::load().map_err(|e| e.to_string())?;
    let client = crate::whats_new::WhatsNewClient::from_config(&cfg).map_err(|e| e.to_string())?;
    client
        .mark_seen(item_id, entry_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Set the OS-level autostart entry and persist the user's preference
/// to `config.toml`. Both writes happen for one toggle so the next
/// boot's setup-closure reconciliation sees the same target value.
#[tauri::command(rename_all = "snake_case")]
pub async fn set_autostart_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())?;
    } else {
        manager.disable().map_err(|e| e.to_string())?;
    }
    let mut cfg = config::load().map_err(|e| e.to_string())?;
    cfg.autostart_enabled = Some(enabled);
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        clamp_timeline_limit, format_session_summary, redact, resolve_client_anon_id,
        resolve_location, run_reparse, search_events_impl, synced_from_sent_at, validate_pair_url,
        EventCount, GameEvent, TimelineEntry, DEFAULT_TIMELINE_LIMIT, MAX_TIMELINE_LIMIT,
    };
    use crate::storage::Storage;
    use tempfile::TempDir;

    /// Build a single TimelineEntry fixture for the session-summary
    /// formatter tests. Synced/raw_line/log_source aren't surfaced by
    /// the summary text, so they get throwaway placeholders.
    fn fixture_timeline_entry(
        id: i64,
        timestamp: &str,
        event_type: &str,
        summary: &str,
    ) -> TimelineEntry {
        TimelineEntry {
            id,
            timestamp: timestamp.to_string(),
            event_type: event_type.to_string(),
            summary: summary.to_string(),
            raw_line: String::new(),
            log_source: "LIVE".to_string(),
            synced: false,
            location: None,
        }
    }

    fn fixed_ts() -> chrono::DateTime<chrono::Utc> {
        // Pin to a known instant so tests don't depend on wall clock.
        // 2026-05-16 14:23:45 UTC -- matches the example in the spec.
        chrono::DateTime::parse_from_rfc3339("2026-05-16T14:23:45Z")
            .expect("parse fixed timestamp")
            .with_timezone(&chrono::Utc)
    }

    /// Phase 3 retro-burst end-to-end test. Seeds a fresh SQLite with a
    /// 5-line `AttachmentReceived` run (matches the
    /// `loadout_restore_burst` rule's min_burst_size of 3), runs
    /// `run_reparse`, and asserts the row count collapsed to 1
    /// `burst_summary` plus the expected stat fields.
    ///
    /// Re-running the same `run_reparse` over the post-collapse state
    /// must be a strict no-op (idempotency invariant).
    #[test]
    fn retro_burst_collapses_attachment_run() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("retro_burst.sqlite3");
        let storage = Storage::open(&path).expect("open storage");

        // Seed 5 AttachmentReceived lines + 1 unrelated line at the end
        // so we can verify the unrelated row survives. Use plausible
        // raw lines that `structural_parse` accepts and that the
        // `loadout_restore_burst` rule matches (event_name
        // `AttachmentReceived` + tag `Inventory`).
        let attachment_line = |i: u64| {
            format!(
                "<2026-05-10T12:00:0{}.000Z> [Notice] <AttachmentReceived> body_{} [Inventory]",
                i, i
            )
        };
        for i in 0..5u64 {
            let line = attachment_line(i);
            let key = format!("seed:LIVE:{}:{}", i * 100, line);
            let key = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, key.as_bytes()).to_string();
            storage
                .insert_event(
                    &key,
                    "attachment_received",
                    &format!("2026-05-10T12:00:0{}.000Z", i),
                    &line,
                    "{}",
                    "LIVE",
                    i * 100,
                )
                .expect("insert attachment");
        }
        let unrelated = "<2026-05-10T12:01:00.000Z> [Notice] <Join PU> address[1.2.3.4] port[1234] shard[pub_x_1_1] locationId[1] [Team_GameServices]";
        let key = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!("seed:LIVE:9999:{}", unrelated).as_bytes(),
        )
        .to_string();
        storage
            .insert_event(
                &key,
                "join_pu",
                "2026-05-10T12:01:00.000Z",
                unrelated,
                "{}",
                "LIVE",
                9999,
            )
            .expect("insert unrelated");

        assert_eq!(storage.total_events().expect("count"), 6);

        // Pass empty remote rules — Phase 3 (retro-burst) is the only
        // path under test; built-in classification on the seed lines is
        // immaterial.
        let stats = run_reparse(&storage, &[]).expect("reparse");
        assert!(stats.error.is_none(), "reparse error: {:?}", stats.error);
        assert_eq!(stats.bursts_collapsed, 1, "expected one burst collapsed");
        assert_eq!(
            stats.members_suppressed, 5,
            "expected all 5 attachment rows suppressed"
        );

        // After collapse: 1 burst_summary + 1 unrelated event = 2 rows.
        assert_eq!(
            storage.total_events().expect("count"),
            2,
            "expected 5 attachments collapsed into 1 summary + 1 unrelated row"
        );

        // Idempotency: running again finds nothing new.
        let stats2 = run_reparse(&storage, &[]).expect("reparse #2");
        assert!(
            stats2.error.is_none(),
            "reparse #2 error: {:?}",
            stats2.error
        );
        assert_eq!(stats2.bursts_collapsed, 0, "second pass must be a no-op");
        assert_eq!(stats2.members_suppressed, 0);
        assert_eq!(storage.total_events().expect("count"), 2);
    }

    /// Seed a fresh sqlite with PlanetTerrainLoad + LocationInventoryRequested +
    /// PlayerDeath, run reparse, and confirm:
    ///   (a) the death event's `zone` is back-filled from the most-recent
    ///       LocationInventoryRequested signal (matches the existing
    ///       enrichment precedence — LIR overwrites PTL because it
    ///       fires later in the seeded order),
    ///   (b) the row's stored metadata blob carries
    ///       `field_provenance["zone"] = InferredFrom { source_event_ids,
    ///       rule_id: "inferred_zone_from_sources" }` pointing at the
    ///       contributing source row's idempotency_key.
    #[test]
    fn zone_enrichment_populates_field_provenance() {
        use starstats_core::metadata::{EventMetadata, FieldProvenance};

        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("zone_enrich_provenance.sqlite3");
        let storage = Storage::open(&path).expect("open storage");

        let ptl_line = "<2026-05-03T18:00:00.000Z> [Notice] <InvalidateAllTerrainCells> Planet OOC_Stanton_2b_Daymar invalidated all terrain cells [Team_3DEngine]";
        let lir_line = "<2026-05-03T18:00:10.000Z> [Notice] <RequestLocationInventory> Player[TheCodeSaiyan] requested inventory for Location[Stanton2_Orison] [Team_CoreGameplayFeatures][Inventory]";
        let pd_line = "<2026-05-03T18:00:20.000Z> [Notice] <Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]> Item 'body_01_noMagicPocket_9754924365641 - Class(body_01_noMagicPocket) - Context(Streamable Runtime-spawned) - Socpak()', Recorded data is: Port Name 'Body_ItemPort', Class GUID: 'dbaa8a7d-755f-4104-8b24-7b58fd1e76f6', KeptId: '9754924365641' [Team_CoreGameplayFeatures][Unknown]";

        let make_key = |source: &str, offset: u64, line: &str| -> String {
            let payload = format!("seed:{source}:{offset}:{line}");
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, payload.as_bytes()).to_string()
        };
        let ptl_key = make_key("LIVE", 0, ptl_line);
        let lir_key = make_key("LIVE", 100, lir_line);
        let pd_key = make_key("LIVE", 200, pd_line);

        storage
            .insert_event(
                &ptl_key,
                "planet_terrain_load",
                "2026-05-03T18:00:00.000Z",
                ptl_line,
                "{}",
                "LIVE",
                0,
            )
            .expect("insert planet terrain load");
        storage
            .insert_event(
                &lir_key,
                "location_inventory_requested",
                "2026-05-03T18:00:10.000Z",
                lir_line,
                "{}",
                "LIVE",
                100,
            )
            .expect("insert location inventory requested");
        storage
            .insert_event(
                &pd_key,
                "player_death",
                "2026-05-03T18:00:20.000Z",
                pd_line,
                "{}",
                "LIVE",
                200,
            )
            .expect("insert player death");

        let stats = run_reparse(&storage, &[]).expect("reparse");
        assert!(stats.error.is_none(), "reparse error: {:?}", stats.error);

        // Read back the player_death row to inspect its enriched
        // payload + metadata. for_each_event walks in id-ASC order; the
        // PD row is the third one.
        let mut pd_row: Option<crate::storage::EventRow> = None;
        storage
            .for_each_event(500, |row| {
                if row.event_type == "player_death" {
                    pd_row = Some(row);
                }
                Ok(())
            })
            .expect("walk events");
        let pd_row = pd_row.expect("player_death row present after reparse");

        // (a) the GameEvent payload was enriched with the LIR zone.
        let event: GameEvent = serde_json::from_str(&pd_row.payload_json).expect("parse payload");
        match event {
            GameEvent::PlayerDeath(d) => {
                assert_eq!(
                    d.zone.as_deref(),
                    Some("Stanton2_Orison"),
                    "expected LIR location to win the most-recent-zone race",
                );
            }
            other => panic!("expected PlayerDeath, got {:?}", other),
        }

        // (b) the row's metadata blob carries InferredFrom provenance
        // pointing at the LIR row's idempotency_key.
        let metadata_json = pd_row
            .metadata_json
            .as_deref()
            .expect("metadata stamped on enriched row");
        let metadata: EventMetadata =
            serde_json::from_str(metadata_json).expect("parse stored metadata");
        let zone_prov = metadata
            .field_provenance
            .get("zone")
            .expect("zone provenance present");
        match zone_prov {
            FieldProvenance::InferredFrom {
                source_event_ids,
                rule_id,
            } => {
                assert_eq!(source_event_ids, &vec![lir_key.clone()]);
                assert_eq!(rule_id, "inferred_zone_from_sources");
            }
            other => panic!("expected InferredFrom, got {:?}", other),
        }
    }

    /// Reparse over a PlayerDeath with no preceding zone signal: the
    /// zone field stays None and no `field_provenance` entry for
    /// `zone` is fabricated. The row's `metadata` cell is left at NULL
    /// because the enrichment pass did nothing.
    #[test]
    fn zone_enrichment_omits_provenance_when_no_zone_signal() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("zone_enrich_no_signal.sqlite3");
        let storage = Storage::open(&path).expect("open storage");

        let pd_line = "<2026-05-03T18:00:00.000Z> [Notice] <Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]> Item 'body_01_noMagicPocket_9754924365641 - Class(body_01_noMagicPocket) - Context(Streamable Runtime-spawned) - Socpak()', Recorded data is: Port Name 'Body_ItemPort', Class GUID: 'dbaa8a7d-755f-4104-8b24-7b58fd1e76f6', KeptId: '9754924365641' [Team_CoreGameplayFeatures][Unknown]";
        let make_key = |source: &str, offset: u64, line: &str| -> String {
            let payload = format!("seed:{source}:{offset}:{line}");
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, payload.as_bytes()).to_string()
        };
        let pd_key = make_key("LIVE", 0, pd_line);
        storage
            .insert_event(
                &pd_key,
                "player_death",
                "2026-05-03T18:00:00.000Z",
                pd_line,
                "{}",
                "LIVE",
                0,
            )
            .expect("insert player death");

        let stats = run_reparse(&storage, &[]).expect("reparse");
        assert!(stats.error.is_none(), "reparse error: {:?}", stats.error);

        let mut pd_row: Option<crate::storage::EventRow> = None;
        storage
            .for_each_event(500, |row| {
                if row.event_type == "player_death" {
                    pd_row = Some(row);
                }
                Ok(())
            })
            .expect("walk events");
        let pd_row = pd_row.expect("player_death row present after reparse");

        let event: GameEvent = serde_json::from_str(&pd_row.payload_json).expect("parse payload");
        match event {
            GameEvent::PlayerDeath(d) => {
                assert!(d.zone.is_none(), "zone must stay None without a signal");
            }
            other => panic!("expected PlayerDeath, got {:?}", other),
        }

        assert!(
            pd_row.metadata_json.is_none(),
            "metadata cell must remain NULL when enrichment fired nothing, got {:?}",
            pd_row.metadata_json,
        );
    }

    #[test]
    fn clamp_timeline_limit_uses_default_when_none() {
        assert_eq!(clamp_timeline_limit(None), DEFAULT_TIMELINE_LIMIT);
    }

    #[test]
    fn clamp_timeline_limit_passes_through_in_range_value() {
        assert_eq!(clamp_timeline_limit(Some(1_000)), 1_000);
    }

    #[test]
    fn clamp_timeline_limit_caps_at_max() {
        assert_eq!(
            clamp_timeline_limit(Some(MAX_TIMELINE_LIMIT * 10)),
            MAX_TIMELINE_LIMIT
        );
    }

    #[test]
    fn clamp_timeline_limit_floor_is_one() {
        // Zero would produce an empty result silently — surface at
        // least one row so the caller can tell the table is non-empty.
        assert_eq!(clamp_timeline_limit(Some(0)), 1);
    }

    #[test]
    fn synced_from_sent_at_pending_when_null() {
        assert!(!synced_from_sent_at(None));
    }

    #[test]
    fn synced_from_sent_at_synced_for_real_timestamp() {
        // `datetime('now')` shape from SQLite.
        assert!(synced_from_sent_at(Some("2026-05-19 14:30:42")));
        // RFC3339 form (other paths may use this).
        assert!(synced_from_sent_at(Some("2026-05-19T14:30:42Z")));
    }

    #[test]
    fn synced_from_sent_at_not_synced_when_quarantined() {
        // Poison-pill sentinel — the row has NOT been accepted
        // server-side; surface as not-synced so the UI counts it as
        // Pending and the recovery banner can act on it.
        assert!(!synced_from_sent_at(Some(
            "__quarantined_2026-05-19 14:30:42"
        )));
    }

    #[test]
    fn synced_from_sent_at_treats_empty_string_as_synced() {
        // Edge case — empty string is technically a non-NULL value
        // and not a `__quarantined_` prefix, so it counts as synced.
        // Real successful sends always write a non-empty datetime so
        // this branch is unreachable in production; assertion locks
        // the contract.
        assert!(synced_from_sent_at(Some("")));
    }

    #[test]
    fn redact_keeps_last_four_chars() {
        assert_eq!(redact("abcdefghij"), "…ghij");
    }

    #[test]
    fn redact_handles_short_input() {
        // Fewer than four chars: just emit what's there. We never call
        // this on empty input (the command rejects empty before
        // redaction) so the "ellipsis only" case is fine.
        assert_eq!(redact("ab"), "…ab");
        assert_eq!(redact(""), "…");
    }

    #[test]
    fn redact_handles_unicode() {
        // Cookie values are ASCII in practice, but `chars` is
        // Unicode-aware so a multibyte tail won't slice mid-codepoint.
        assert_eq!(redact("hello🚀✨"), "…lo🚀✨");
    }

    #[test]
    fn validate_pair_url_accepts_https() {
        assert!(validate_pair_url("https://api.example.com").is_ok());
        assert!(validate_pair_url("https://api.example.com:8443/api").is_ok());
    }

    #[test]
    fn validate_pair_url_accepts_localhost_http() {
        assert!(validate_pair_url("http://localhost:3000").is_ok());
        assert!(validate_pair_url("http://127.0.0.1:8080").is_ok());
    }

    #[test]
    fn validate_pair_url_rejects_remote_http() {
        assert!(validate_pair_url("http://api.example.com").is_err());
        assert!(validate_pair_url("http://attacker.example/").is_err());
    }

    #[test]
    fn validate_pair_url_rejects_hostile_schemes() {
        assert!(validate_pair_url("javascript:alert(1)").is_err());
        assert!(validate_pair_url("file:///etc/passwd").is_err());
        assert!(validate_pair_url("data:text/html,<script>").is_err());
        assert!(validate_pair_url("").is_err());
    }

    #[test]
    fn validate_pair_url_rejects_https_without_host() {
        assert!(validate_pair_url("https://").is_err());
    }

    /// Two back-to-back `cached_sysinfo` calls land microseconds apart,
    /// well inside the 60s TTL. The cached path must return the exact
    /// same tuple — proves the cache is being read on the second call
    /// rather than recomputed.
    #[test]
    fn cached_sysinfo_hits_within_ttl() {
        let first = super::cached_sysinfo();
        let second = super::cached_sysinfo();
        assert_eq!(
            first, second,
            "cached call within TTL must return identical values"
        );
    }

    #[test]
    fn session_summary_empty_returns_no_events_line() {
        let out = format_session_summary(&[], &[], fixed_ts());
        assert!(
            out.contains("No events captured yet."),
            "empty summary should call out zero events, got:\n{out}"
        );
        assert!(out.starts_with("StarStats — session summary"));
    }

    #[test]
    fn session_summary_lists_top_types_in_order_and_count() {
        let counts = vec![
            EventCount {
                event_type: "login".to_string(),
                count: 234,
            },
            EventCount {
                event_type: "ship_destroyed".to_string(),
                count: 89,
            },
            EventCount {
                event_type: "location_enter".to_string(),
                count: 67,
            },
        ];
        let out = format_session_summary(&counts, &[], fixed_ts());
        // Order: login must appear before ship_destroyed which must
        // appear before location_enter.
        let login_idx = out.find("login").expect("login present");
        let ship_idx = out.find("ship_destroyed").expect("ship_destroyed present");
        let loc_idx = out.find("location_enter").expect("location_enter present");
        assert!(
            login_idx < ship_idx,
            "login should come before ship_destroyed"
        );
        assert!(
            ship_idx < loc_idx,
            "ship_destroyed should come before location_enter"
        );
        // Counts (comma-formatted) must show up.
        assert!(out.contains("234"), "count 234 missing: {out}");
        assert!(out.contains("89"), "count 89 missing: {out}");
        assert!(out.contains("67"), "count 67 missing: {out}");
    }

    #[test]
    fn session_summary_caps_top_types_at_ten() {
        let counts: Vec<EventCount> = (0..15)
            .map(|i| EventCount {
                event_type: format!("type_{:02}", i),
                count: (100 - i) as u64,
            })
            .collect();
        let out = format_session_summary(&counts, &[], fixed_ts());
        // First 10 should appear, indices 10..15 should not.
        for i in 0..10 {
            let name = format!("type_{:02}", i);
            assert!(out.contains(&name), "expected {name} in output: {out}");
        }
        for i in 10..15 {
            let name = format!("type_{:02}", i);
            assert!(
                !out.contains(&name),
                "did not expect {name} in capped output: {out}"
            );
        }
    }

    #[test]
    fn session_summary_caps_timeline_at_twenty() {
        // 25 entries, newest first (matches what storage::recent_events
        // returns). Each entry's summary embeds its index so we can
        // check which made the cut.
        let timeline: Vec<TimelineEntry> = (0..25)
            .map(|i| {
                fixture_timeline_entry(
                    i as i64,
                    "2026-05-16T14:00:00Z",
                    "test_event",
                    &format!("summary_{:02}", i),
                )
            })
            .collect();
        let counts = vec![EventCount {
            event_type: "test_event".to_string(),
            count: 25,
        }];
        let out = format_session_summary(&counts, &timeline, fixed_ts());
        // First 20 summaries (indices 0..20) must appear; 20..25 must not.
        for i in 0..20 {
            let s = format!("summary_{:02}", i);
            assert!(out.contains(&s), "expected {s} in timeline output: {out}");
        }
        for i in 20..25 {
            let s = format!("summary_{:02}", i);
            assert!(
                !out.contains(&s),
                "did not expect {s} in capped timeline: {out}"
            );
        }
    }

    #[test]
    fn session_summary_timestamp_header_present() {
        let out = format_session_summary(&[], &[], fixed_ts());
        assert!(
            out.starts_with("StarStats — session summary"),
            "must lead with the title, got: {out}"
        );
        // Find the "Generated " line and confirm a 4-digit year follows.
        let idx = out.find("Generated ").expect("Generated label");
        let tail = &out[idx + "Generated ".len()..];
        let year_part: String = tail.chars().take(4).collect();
        assert_eq!(
            year_part.len(),
            4,
            "expected at least 4 chars after 'Generated ', got: {tail}"
        );
        assert!(
            year_part.chars().all(|c| c.is_ascii_digit()),
            "first 4 chars after 'Generated ' should be a year, got: {year_part}"
        );
    }

    #[test]
    fn resolve_client_anon_id_returns_existing_when_set() {
        // Stable: the same persisted value comes back, no dirty flag,
        // no rewrite of config.toml on subsequent calls.
        let (id, dirty) = resolve_client_anon_id(Some("anon_existing_abc"));
        assert_eq!(id, "anon_existing_abc");
        assert!(!dirty, "existing id should not flag the config dirty");
    }

    #[test]
    fn resolve_client_anon_id_generates_when_missing() {
        // First-call path: produces a fresh anon_<uuid simple> string
        // and flags the config as dirty so the caller persists it.
        let (id, dirty) = resolve_client_anon_id(None);
        assert!(dirty, "missing id must flag the config dirty");
        assert!(
            id.starts_with("anon_"),
            "id should be prefixed with anon_, got: {id}"
        );
        // uuid simple is 32 hex chars.
        let hex = id.strip_prefix("anon_").unwrap();
        assert_eq!(hex.len(), 32, "expected 32-char uuid simple, got: {hex}");
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "id tail should be hex, got: {hex}"
        );
    }

    #[test]
    fn resolve_client_anon_id_regenerates_when_blank() {
        // A corrupted / blank config should self-heal rather than send
        // an empty string to the server (which the route rejects with
        // 400 `invalid_client_anon_id`).
        for blank in ["", "   ", "\n\t"] {
            let (id, dirty) = resolve_client_anon_id(Some(blank));
            assert!(dirty, "blank id ({blank:?}) must trigger regeneration");
            assert!(id.starts_with("anon_"), "regenerated id: {id}");
        }
    }

    // ─── search_events_impl pagination + has_more ─────────────────

    /// Seed the storage with 5 events spanning two event_types so the
    /// command-layer tests have enough rows to paginate and filter.
    fn seed_command_search_corpus(storage: &Storage) {
        let rows: &[(&str, &str, &str, &str)] = &[
            (
                "Ship_Destroyed",
                "2026-05-01T10:00:00Z",
                r#"{"ship":"alpha"}"#,
                "raw a",
            ),
            (
                "Player_Joined",
                "2026-05-01T11:00:00Z",
                r#"{"who":"bravo"}"#,
                "raw b",
            ),
            (
                "Ship_Destroyed",
                "2026-05-01T12:00:00Z",
                r#"{"ship":"charlie"}"#,
                "raw c",
            ),
            (
                "Player_Joined",
                "2026-05-01T13:00:00Z",
                r#"{"who":"delta"}"#,
                "raw d",
            ),
            (
                "Ship_Destroyed",
                "2026-05-01T14:00:00Z",
                r#"{"ship":"echo"}"#,
                "raw e",
            ),
        ];
        for (i, (ty, ts, payload, raw)) in rows.iter().enumerate() {
            let key = format!("cmd-search-seed-{i}");
            storage
                .insert_event(&key, ty, ts, raw, payload, "live", i as u64)
                .expect("seed insert");
        }
    }

    fn fresh_command_storage() -> (Storage, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("cmd_search.sqlite3");
        let storage = Storage::open(&path).expect("open storage");
        (storage, dir)
    }

    #[test]
    fn resolve_location_none_for_placeless_event() {
        let cat = starstats_core::location_catalog::LocationCatalog::from_entries(vec![]);
        let ev = GameEvent::LegacyLogin(starstats_core::events::LegacyLogin {
            timestamp: "t".into(),
            handle: "h".into(),
            server_time: None,
        });
        assert!(resolve_location(&ev, &cat).is_none());
    }

    #[test]
    fn resolve_location_name_without_slug_on_catalog_miss() {
        // Empty catalogue → the classifier's heuristic still recovers a
        // friendly name + system from the engine string, but no slug,
        // so the UI shows text without a KB link ("best name, link only
        // when confident").
        let cat = starstats_core::location_catalog::LocationCatalog::from_entries(vec![]);
        let ev = GameEvent::PlanetTerrainLoad(starstats_core::events::PlanetTerrainLoad {
            timestamp: "t".into(),
            planet: "OOC_Stanton_2b_Daymar".into(),
        });
        let r = resolve_location(&ev, &cat).expect("location resolved");
        assert!(r.slug.is_none(), "no slug on catalog miss");
        assert!(!r.display_name.is_empty());
        assert_eq!(r.system.as_deref(), Some("Stanton"));
    }

    #[test]
    fn search_events_impl_empty_query_returns_all_rows() {
        let (storage, _tmp) = fresh_command_storage();
        seed_command_search_corpus(&storage);
        let res = search_events_impl(&storage, None, None, None, 100).expect("search");
        assert_eq!(res.entries.len(), 5);
        assert_eq!(res.total, 5);
        assert!(!res.has_more, "all rows returned, has_more must be false");
        // Newest first.
        let ids: Vec<i64> = res.entries.iter().map(|e| e.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(ids, sorted);
    }

    #[test]
    fn search_events_impl_query_matches_subset() {
        let (storage, _tmp) = fresh_command_storage();
        seed_command_search_corpus(&storage);
        // "charlie" only appears in one payload.
        let res = search_events_impl(&storage, Some("charlie"), None, None, 100).expect("search");
        assert_eq!(res.entries.len(), 1);
        assert_eq!(res.total, 1);
        assert!(!res.has_more);
    }

    #[test]
    fn search_events_impl_before_id_pages_correctly() {
        let (storage, _tmp) = fresh_command_storage();
        seed_command_search_corpus(&storage);
        // Page 1: limit 2 of 5 -> has_more true.
        let page1 = search_events_impl(&storage, None, None, None, 2).expect("page1");
        assert_eq!(page1.entries.len(), 2);
        assert_eq!(page1.total, 5);
        assert!(page1.has_more);

        let cursor = page1.entries.last().unwrap().id;
        let page2 = search_events_impl(&storage, None, None, Some(cursor), 2).expect("page2");
        assert_eq!(page2.entries.len(), 2);
        // Page 2 rows strictly older than the cursor.
        for e in &page2.entries {
            assert!(e.id < cursor);
        }
        // No overlap with page 1.
        let p1_ids: std::collections::HashSet<i64> = page1.entries.iter().map(|e| e.id).collect();
        for e in &page2.entries {
            assert!(!p1_ids.contains(&e.id));
        }
        // After page 2 (4 of 5 total surfaced) there's still 1 left,
        // but page2 is full (limit=2) and total > returned-so-far in
        // this page, so has_more is still true.
        assert!(page2.has_more);

        // Page 3: limit 2, but only 1 row remains -> has_more false.
        let cursor2 = page2.entries.last().unwrap().id;
        let page3 = search_events_impl(&storage, None, None, Some(cursor2), 2).expect("page3");
        assert_eq!(page3.entries.len(), 1);
        assert!(!page3.has_more, "partial final page must clear has_more");
    }

    #[test]
    fn search_events_impl_empty_strings_treated_as_none() {
        let (storage, _tmp) = fresh_command_storage();
        seed_command_search_corpus(&storage);
        // Empty-string query + empty-string type_filter should behave
        // identically to None/None — saves the front end from having
        // to nullify "" before sending.
        let baseline = search_events_impl(&storage, None, None, None, 100).expect("baseline");
        let empties = search_events_impl(&storage, Some(""), Some(""), None, 100).expect("empties");
        assert_eq!(baseline.entries.len(), empties.entries.len());
        assert_eq!(baseline.total, empties.total);
        let b_ids: Vec<i64> = baseline.entries.iter().map(|e| e.id).collect();
        let e_ids: Vec<i64> = empties.entries.iter().map(|e| e.id).collect();
        assert_eq!(b_ids, e_ids);
    }

    #[test]
    fn search_events_impl_has_more_false_when_total_equals_returned() {
        let (storage, _tmp) = fresh_command_storage();
        seed_command_search_corpus(&storage);
        // limit exactly equals total -> page is full but nothing left.
        let res = search_events_impl(&storage, None, None, None, 5).expect("search");
        assert_eq!(res.entries.len(), 5);
        assert_eq!(res.total, 5);
        assert!(!res.has_more);
    }

    /// Guards against the re-parse-path bug where the rule_id comparison
    /// used the bare "loadout_restore" string instead of the actual rule id
    /// "loadout_restore_burst", causing the re-parse path to always skip the
    /// kind/categories branch and produce a burst summary with kind=None.
    ///
    /// This test drives the full `run_reparse` path (Phase 3 retro-burst) with
    /// 5 real `AttachmentReceived [Inventory]` lines whose bodies contain
    /// parseable item class names, then asserts:
    ///   (a) the collapsed burst_summary row has `kind == "loadout_restore"`, and
    ///   (b) the categories map is non-empty (items were classified).
    ///
    /// The test WOULD FAIL against the old `commands.rs` literal `== "loadout_restore"`
    /// because the rule emits `rule_id = "loadout_restore_burst"`, so the branch
    /// was never entered and the stored payload had `kind: null`.
    #[test]
    fn retro_burst_reparse_sets_kind_and_categories_for_loadout_rule() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("retro_burst_kind.sqlite3");
        let storage = Storage::open(&path).expect("open storage");

        // Use real AttachmentReceived log lines with varied item classes so
        // build_loadout_categories produces a non-empty map.  The format is
        // taken verbatim from parser.rs test fixtures (see parser.rs:1068).
        let lines = [
            "<2026-05-10T12:00:00.000Z> [Notice] <AttachmentReceived> Player[TestPilot] Attachment[rsi_odyssey_undersuit_01_01_01_200000000001, rsi_odyssey_undersuit_01_01_01, 200000000001] Status[persistent] Port[Armor_Undersuit] Elapsed[1.0] [Team_CoreGameplayFeatures][Inventory]",
            "<2026-05-10T12:00:00.001Z> [Notice] <AttachmentReceived> Player[TestPilot] Attachment[rsi_odyssey_helmet_01_01_01_200000000002, rsi_odyssey_helmet_01_01_01, 200000000002] Status[persistent] Port[Armor_Helmet] Elapsed[1.1] [Team_CoreGameplayFeatures][Inventory]",
            "<2026-05-10T12:00:00.002Z> [Notice] <AttachmentReceived> Player[TestPilot] Attachment[rsi_odyssey_arms_01_01_01_200000000003, rsi_odyssey_arms_01_01_01, 200000000003] Status[persistent] Port[Armor_Arms] Elapsed[1.2] [Team_CoreGameplayFeatures][Inventory]",
            "<2026-05-10T12:00:00.003Z> [Notice] <AttachmentReceived> Player[TestPilot] Attachment[grin_multitool_01_tractorbeam_200000000004, grin_multitool_01_tractorbeam, 200000000004] Status[persistent] Port[WEAPON_RIGHT] Elapsed[1.3] [Team_CoreGameplayFeatures][Inventory]",
            "<2026-05-10T12:00:00.004Z> [Notice] <AttachmentReceived> Player[TestPilot] Attachment[rsi_p4ar_01_200000000005, rsi_p4ar_01, 200000000005] Status[persistent] Port[WEAPON_LEFT] Elapsed[1.4] [Team_CoreGameplayFeatures][Inventory]",
        ];

        for (i, line) in lines.iter().enumerate() {
            let key = uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_OID,
                format!("seed:LIVE:{}:{}", i * 100, line).as_bytes(),
            )
            .to_string();
            storage
                .insert_event(
                    &key,
                    "attachment_received",
                    &format!("2026-05-10T12:00:00.{:03}Z", i),
                    line,
                    "{}",
                    "LIVE",
                    (i * 100) as u64,
                )
                .expect("insert attachment");
        }

        assert_eq!(storage.total_events().expect("count"), 5);

        let stats = run_reparse(&storage, &[]).expect("reparse");
        assert!(stats.error.is_none(), "reparse error: {:?}", stats.error);
        assert_eq!(stats.bursts_collapsed, 1, "expected one burst collapsed");

        // The burst_summary row must carry kind="loadout_restore" and
        // non-empty categories.  Fetch via search_events_impl and parse
        // the payload.
        // Fetch all remaining events (member rows deleted, only the summary remains).
        // Use type_filter="burst_summary" (second arg to search_events_impl) to
        // select only the collapsed row.
        let results = search_events_impl(&storage, None, Some("burst_summary"), None, 10)
            .expect("search burst_summary");
        assert_eq!(
            results.entries.len(),
            1,
            "expected exactly one burst_summary row; got entries: {:?}",
            results
                .entries
                .iter()
                .map(|e| &e.event_type)
                .collect::<Vec<_>>()
        );

        // search_events_impl confirmed exactly one burst_summary row exists.
        // Read its payload directly from storage (for_each_event exposes the
        // full payload_json column) and assert kind + categories.
        let mut found_kind: Option<String> = None;
        let mut found_categories: Option<std::collections::HashMap<String, u64>> = None;
        storage
            .for_each_event(100, |row| {
                if row.event_type == "burst_summary" {
                    // payload_json column holds the full GameEvent JSON
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&row.payload_json) {
                        found_kind = v
                            .get("kind")
                            .and_then(|k| k.as_str())
                            .map(|s| s.to_string());
                        if let Some(cats_v) = v.get("categories") {
                            if let Ok(cats) = serde_json::from_value::<
                                std::collections::HashMap<String, u64>,
                            >(cats_v.clone())
                            {
                                found_categories = Some(cats);
                            }
                        }
                    }
                }
                Ok(())
            })
            .expect("for_each_event");

        assert_eq!(
            found_kind.as_deref(),
            Some("loadout_restore"),
            "re-parse path must set kind=loadout_restore; got: {:?}",
            found_kind
        );
        let cats = found_categories.expect("categories must be non-empty for a loadout burst");
        assert!(
            !cats.is_empty(),
            "categories map must be non-empty; got empty map"
        );

        // TDD: items must be present in the reparse path too, one per member line,
        // each carrying a non-empty port and category.
        let mut found_items: Option<Vec<serde_json::Value>> = None;
        storage
            .for_each_event(100, |row| {
                if row.event_type == "burst_summary" {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&row.payload_json) {
                        if let Some(arr) = v.get("items").and_then(|i| i.as_array()) {
                            found_items = Some(arr.clone());
                        }
                    }
                }
                Ok(())
            })
            .expect("for_each_event (items check)");

        let items = found_items.expect("reparse burst_summary must include an 'items' array");
        assert_eq!(
            items.len(),
            5,
            "items must have one entry per AttachmentReceived member (5 lines); got {items:?}",
        );
        for item in &items {
            let port = item
                .get("port")
                .and_then(|v| v.as_str())
                .expect("each item must have a port string");
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
        // Spot-check: one item must have port == "Armor_Helmet".
        assert!(
            items
                .iter()
                .any(|it| it.get("port").and_then(|v| v.as_str()) == Some("Armor_Helmet")),
            "expected one item with port == 'Armor_Helmet'; got {items:?}",
        );
    }

    /// Const-seam test: asserts the loadout burst rule id constant matches
    /// what `builtin_burst_rules()` emits for the loadout rule.
    ///
    /// This is the minimal guard against a 3rd drift: if someone renames the
    /// rule id in burst_rules.rs but forgets to update the const (or vice
    /// versa), this test fails immediately.  The retro_burst_reparse test
    /// above fails on behaviour; this one fails on contract.
    #[test]
    fn loadout_burst_rule_id_const_matches_builtin_rule() {
        use crate::burst_rules::{builtin_burst_rules, LOADOUT_RESTORE_BURST_RULE_ID};
        let rules = builtin_burst_rules();
        let loadout = rules
            .iter()
            .find(|r| r.id == LOADOUT_RESTORE_BURST_RULE_ID)
            .expect("builtin_burst_rules() must contain a rule whose id equals LOADOUT_RESTORE_BURST_RULE_ID");
        // Belt-and-suspenders: the literal value the const holds.
        assert_eq!(loadout.id, "loadout_restore_burst");
        assert_eq!(LOADOUT_RESTORE_BURST_RULE_ID, "loadout_restore_burst");
    }
}
