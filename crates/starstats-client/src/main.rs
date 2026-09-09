//! StarStats tray client — Tauri 2 host process.
//!
//! Wiring:
//!   - SQLite store opened in user data dir
//!   - Game.log discovery → start tail loop on first match
//!   - Tray icon with Show / Quit
//!   - Tauri commands exposed to the React frontend

// Detach the console window in release builds — this is a tray app,
// users launching from the Start menu don't expect a flashing cmd
// window. Stdout/stderr are silenced as a side-effect; persistent
// diagnostics live in the panic hook + the optional `debug_logging`
// file appender (see `init_telemetry`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backfill;
mod burst_rules;
mod cloud_sync;
mod commands;
mod config;
mod config_sync;
mod crashes;
mod discovery;
mod gamelog;
mod health;
mod launcher;
mod location_catalog;
// Opt-in connector to a self-hosted org platform (the `orgplatform`
// companion project). Forwards read-only presence telemetry derived
// from already-parsed Game.log events; spawned from the setup closure
// below only when the user opts in via `[org_connector]` in config.toml.
mod org_connector;
// Tray-side hangar fetcher (Wave 5b). Spawned from the Tauri setup
// closure below when an api_url + access_token are configured.
mod hangar;
// Foundational layer for tray-side hangar fetching (Wave 5b).
// `process_guard` is consumed by `hangar` (kept here as a
// first-party module so the binary's trust scope is explicit);
// `secret` is consumed by both `hangar` and the cookie-management
// commands.
mod parser_defs;
mod preferences_client;
mod probes;
#[allow(dead_code)]
mod process_guard;
mod secret;
mod state;
mod storage;
mod sync;
mod whats_new;

use crate::hangar::HangarStats;
use crate::state::{AccountStatus, AppState};
use crate::storage::Storage;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Registry};

fn main() {
    let debug_logging = config::load().map(|c| c.debug_logging).unwrap_or(false);
    init_telemetry(debug_logging);
    install_panic_hook();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "starstats-client starting"
    );

    tauri::Builder::default()
        // Intercept the main window's close button — without this,
        // closing the window destroys the only webview and Tauri's
        // default "exit on last window close" kicks in, killing the
        // app instead of leaving the tray icon resident. Hide instead;
        // Quit is reachable from the tray menu.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        // Autostart plugin — writes the per-OS launch-on-sign-in entry
        // (HKCU Run on Windows, XDG autostart on Linux, LaunchAgent on
        // macOS). `--autostart` lets the setup closure tell a boot-time
        // launch apart from a user-initiated one (see `is_autostart_launch`).
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            // 0. Updater plugin — desktop only. Always registered so
            //    the manual "Check for updates" command in the Settings
            //    pane works regardless of the auto-check preference.
            //    The startup auto-check below is gated on the user's
            //    `auto_update_check` config flag.
            #[cfg(desktop)]
            {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;

                let cfg = config::load().unwrap_or_default();
                if cfg.auto_update_check {
                    use tauri_plugin_updater::UpdaterExt;
                    let handle = app.handle().clone();
                    let current_version = env!("CARGO_PKG_VERSION");
                    let channel = cfg.release_channel;
                    tauri::async_runtime::spawn(async move {
                        // Override the static `endpoints` from
                        // tauri.conf.json with the user's selected
                        // channel. `updater_builder()` lets us swap
                        // endpoints per-call, so changing the channel
                        // in Settings takes effect on the next check
                        // without an app restart.
                        let url = match channel.manifest_url().parse::<tauri::Url>() {
                            Ok(u) => u,
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    channel = channel.as_str(),
                                    "release channel URL did not parse — skipping check"
                                );
                                return;
                            }
                        };
                        let updater = match handle.updater_builder().endpoints(vec![url]) {
                            Ok(b) => match b.build() {
                                Ok(u) => u,
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        channel = channel.as_str(),
                                        current_version = current_version,
                                        "could not build updater for channel"
                                    );
                                    return;
                                }
                            },
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    channel = channel.as_str(),
                                    current_version = current_version,
                                    "could not set updater endpoints"
                                );
                                return;
                            }
                        };
                        match updater.check().await {
                            Ok(Some(update)) => tracing::info!(
                                channel = channel.as_str(),
                                current_version = current_version,
                                new_version = %update.version,
                                "starstats update available"
                            ),
                            Ok(None) => tracing::info!(
                                channel = channel.as_str(),
                                current_version = current_version,
                                "starstats is up to date"
                            ),
                            Err(e) => tracing::warn!(
                                error = %e,
                                channel = channel.as_str(),
                                current_version = current_version,
                                "updater check failed"
                            ),
                        }
                    });
                } else {
                    tracing::info!("startup updater check skipped (auto_update_check=false)");
                }
            }

            // 0a. Autostart reconciliation. Default-on: first launch
            //     enables the per-OS sign-in entry, then persists the
            //     resolved state so we never re-trigger the "first run
            //     = enable" path on subsequent boots. User-driven
            //     changes flow through `set_autostart_enabled` and
            //     write the same field, so the toggle state and the
            //     OS entry stay in sync.
            #[cfg(desktop)]
            {
                use tauri_plugin_autostart::ManagerExt;

                let manager = app.autolaunch();
                let mut cfg = config::load().unwrap_or_default();
                let target = cfg.autostart_enabled.unwrap_or(true);
                match (target, manager.is_enabled().unwrap_or(false)) {
                    (true, false) => {
                        if let Err(e) = manager.enable() {
                            tracing::warn!(error = %e, "failed to enable autostart");
                        }
                    }
                    (false, true) => {
                        if let Err(e) = manager.disable() {
                            tracing::warn!(error = %e, "failed to disable autostart");
                        }
                    }
                    _ => {}
                }
                if cfg.autostart_enabled.is_none() {
                    cfg.autostart_enabled = Some(target);
                    if let Err(e) = config::save(&cfg) {
                        tracing::warn!(error = %e, "failed to persist autostart preference");
                    }
                }
            }

            // 1. Local SQLite store
            let storage_path = config::data_dir()?.join("data.sqlite3");
            let storage = Arc::new(Storage::open(&storage_path)?);
            tracing::info!(path = %storage_path.display(), "opened local store");

            // Hydrate the parser-definition cache from sqlite before
            // any ingest spawns — guarantees the first events through
            // the tail benefit from any rules cached on the previous
            // run, even if the network fetch hasn't landed yet.
            let parser_def_cache = parser_defs::RuleCache::new();
            parser_defs::hydrate_from_storage(&storage, &parser_def_cache);
            // Read the v2 event-metadata flag once at startup. The
            // value is captured into the tail + backfill workers; a
            // mid-session toggle in config.toml requires an app
            // restart, which matches how every other parser-affecting
            // setting in this codebase behaves.
            let enable_v2_metadata = config::load()
                .map(|c| c.v2_metadata_enabled())
                .unwrap_or(false);
            // Captured once at startup alongside `enable_v2_metadata`, same
            // restart-to-change convention. Empty when unpaired; it's the
            // own-handle PII input for the unknown-line capture pipeline, so
            // an empty value simply means no own-handle redaction is offered
            // until the tray is paired and restarted.
            let own_handle = config::load()
                .ok()
                .and_then(|c| c.remote_sync.claimed_handle.clone())
                .unwrap_or_default();
            // Spawn the network refresher on the Tauri runtime so it
            // doesn't need a local tokio context. 6h cadence; first
            // tick runs immediately so an online cold-start picks up
            // the active manifest.
            if let Some(api_url) = config::load()
                .ok()
                .and_then(|c| c.remote_sync.api_url.clone())
            {
                let storage_for_fetch = Arc::clone(&storage);
                let cache_for_fetch = parser_def_cache.clone();
                tauri::async_runtime::spawn(parser_defs::run_fetcher(
                    api_url,
                    storage_for_fetch,
                    cache_for_fetch,
                ));
            }

            // 2. Live tail stats holder
            let tail_stats = Arc::new(parking_lot::Mutex::new(gamelog::TailStats::default()));
            let sync_stats = Arc::new(parking_lot::Mutex::new(sync::SyncStats::default()));
            let hangar_stats: Arc<parking_lot::Mutex<HangarStats>> =
                Arc::new(parking_lot::Mutex::new(HangarStats::default()));
            let account_status = Arc::new(parking_lot::Mutex::new(AccountStatus::default()));
            let sync_kick = Arc::new(sync::SyncKick::default());
            let hangar_kick = Arc::new(tokio::sync::Notify::new());
            // Fired by the Game.log tail after every drain that ingested
            // events; awaited by the opt-in org-platform connector so it
            // forwards presence the instant it lands, not on a 3s poll.
            let tail_event_kick = Arc::new(tokio::sync::Notify::new());

            // Location catalogue for the client-side classifier. Prefer
            // the persisted snapshot from the last `/v1/reference/location`
            // fetch; fall back to the bundled bootstrap. Hot-swapped after
            // future fetches via the `location_catalog` RwLock. Built here
            // — BEFORE the sync workers — because the sync drain classifies
            // each event's location against this snapshot before shipping
            // it (`build_batch` → `classify`), so the workers need the
            // handle.
            let location_catalog_state = {
                let snapshot_path = config::data_dir()
                    .ok()
                    .map(|d| d.join("location_catalog.json"));
                let cat = location_catalog::resolve_catalog(snapshot_path.as_deref());
                Arc::new(parking_lot::RwLock::new(Arc::new(cat)))
            };
            // Clone the handle before the state moves into Tauri, so the
            // background refresh below can hot-swap the catalogue.
            let catalog_refresh_handle = Arc::clone(&location_catalog_state);
            // Second clone for the opt-in org-platform connector, which
            // classifies each forwarded event's location against the same
            // (hot-swappable) catalogue the sync drain uses.
            let catalog_for_org = Arc::clone(&location_catalog_state);

            // 2a/2b/2c. Sync worker + account-status hydration +
            //           hangar refresh worker. The sync handle is
            //           stashed into AppState so `save_config` and
            //           `redeem_pair` can abort+respawn this worker
            //           when the user toggles `remote_sync.enabled`
            //           or pairs a new device.
            let initial_sync_handle = start_sync_workers(
                Arc::clone(&storage),
                Arc::clone(&sync_stats),
                Arc::clone(&hangar_stats),
                Arc::clone(&account_status),
                Arc::clone(&sync_kick),
                Arc::clone(&hangar_kick),
                app.handle().clone(),
                Arc::clone(&location_catalog_state),
            );
            let sync_handle = Arc::new(parking_lot::Mutex::new(initial_sync_handle));

            // 3. Discover Game.log and start tailing the most recently
            //    modified one (LIVE if the user just played).
            let watcher = start_log_tail(
                Arc::clone(&storage),
                Arc::clone(&tail_stats),
                parser_def_cache.clone(),
                enable_v2_metadata,
                own_handle.clone(),
                Arc::clone(&tail_event_kick),
            )?;

            // 3a/3b/3c. Background workers — launcher tail, crash-dir
            //     scanner, rotated-log backfill. Each is wrapped in
            //     `catch_unwind` so a panic in any one of them doesn't
            //     take down the whole app. Defense-in-depth — the
            //     panic hook still captures the trace before unwinding.
            let launcher_stats =
                Arc::new(parking_lot::Mutex::new(launcher::LauncherStats::default()));
            let launcher_storage = Arc::clone(&storage);
            let launcher_stats_clone = Arc::clone(&launcher_stats);
            let launcher_watcher = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || {
                    tauri::async_runtime::block_on(async move {
                        launcher::start_tail(launcher_storage, launcher_stats_clone).await
                    })
                },
            )) {
                Ok(Ok(w)) => w,
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "launcher tail start failed; continuing without it");
                    None
                }
                Err(_) => {
                    tracing::error!(
                        "launcher::start_tail PANICKED; continuing without launcher tail"
                    );
                    None
                }
            };

            let crash_stats = Arc::new(parking_lot::Mutex::new(crashes::CrashStats::default()));
            let crash_storage = Arc::clone(&storage);
            let crash_stats_clone = Arc::clone(&crash_stats);
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crashes::spawn_scanner(crash_storage, crash_stats_clone);
            }))
            .is_err()
            {
                tracing::error!(
                    "crashes::spawn_scanner PANICKED; continuing without crash scanning"
                );
            }

            let backfill_stats =
                Arc::new(parking_lot::Mutex::new(backfill::BackfillStats::default()));
            let backfill_storage = Arc::clone(&storage);
            let backfill_stats_clone = Arc::clone(&backfill_stats);
            let backfill_rules = parser_def_cache.clone();
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                backfill::spawn(
                    backfill_storage,
                    backfill_stats_clone,
                    backfill_rules,
                    enable_v2_metadata,
                    own_handle.clone(),
                );
            }))
            .is_err()
            {
                tracing::error!(
                    "backfill::spawn PANICKED; continuing without rotated-log backfill"
                );
            }

            // 3d. Opt-in org-platform connector. Rides the same read-only
            //     Game.log boundary as the sync worker — it forwards
            //     presence telemetry derived from already-parsed events to
            //     a self-hosted org platform's /ws/member channel. No-op
            //     unless the user set `[org_connector] enabled = true`
            //     with a platform_url + bearer_token. The handle is stored
            //     in AppState so `save_config` can abort + respawn the
            //     connector when the user changes the settings without
            //     requiring an app restart.
            let org_connector_handle = {
                let org_storage = Arc::clone(&storage);
                let org_cfg = config::load().map(|c| c.org_connector).unwrap_or_default();
                let handle = org_connector::spawn(
                    org_storage,
                    catalog_for_org,
                    org_cfg,
                    Arc::clone(&tail_event_kick),
                );
                Arc::new(parking_lot::Mutex::new(handle))
            };

            app.manage(AppState {
                storage,
                location_catalog: location_catalog_state,
                tail_stats,
                sync_stats,
                hangar_stats,
                account_status,
                sync_kick,
                hangar_kick,
                tail_event_kick,
                launcher_stats,
                crash_stats,
                backfill_stats,
                parser_def_cache,
                sync_handle,
                org_connector_handle,
                _tail_handle: parking_lot::Mutex::new(watcher),
                _launcher_handle: parking_lot::Mutex::new(launcher_watcher),
                update_available: Arc::new(parking_lot::Mutex::new(None)),
            });

            // Background: upgrade the location catalogue from the bundled
            // 15-row bootstrap to the full server set (~1955 rows),
            // persist a snapshot for next launch, and hot-swap the
            // in-memory catalogue so the classifier links the long tail of
            // locations without an app restart. Best-effort — failure
            // leaves the bootstrap/snapshot in place.
            if let Some(api_url) = config::load()
                .ok()
                .and_then(|c| c.remote_sync.api_url.clone())
            {
                if let Some(snapshot_path) = config::data_dir()
                    .ok()
                    .map(|d| d.join("location_catalog.json"))
                {
                    let handle = catalog_refresh_handle;
                    tauri::async_runtime::spawn(async move {
                        match location_catalog::fetch_and_persist(&api_url, &snapshot_path).await {
                            Ok(entries) => {
                                let n = entries.len();
                                *handle.write() = Arc::new(
                                    starstats_core::location_catalog::LocationCatalog::from_entries(
                                        entries,
                                    ),
                                );
                                tracing::info!(
                                    entries = n,
                                    "location catalogue refreshed from server"
                                );
                            }
                            Err(e) => tracing::warn!(
                                error = %e,
                                "location catalogue refresh failed; using bootstrap/snapshot"
                            ),
                        }
                    });
                }
            }

            // 4. Tray icon + menu
            build_tray(app)?;

            // 5. Show the main window on first launch — unless this
            //    process was started by the OS via the autostart hook,
            //    in which case the tray icon is enough and the window
            //    stays hidden until the user clicks it.
            if !is_autostart_launch() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                }
            }

            // 6. One-shot preferences pull on app launch. Reconciles
            //    any drift from another device since last shutdown.
            //    Always reloads config from disk so we have the fully
            //    merged state (autostart reconciliation above may have
            //    written a new value).
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let cfg = match crate::config::load() {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(error = %e, "launch pull: config load failed; skipping");
                            return;
                        }
                    };
                    match crate::sync::piggyback_preferences_pull(&cfg).await {
                        crate::sync::PiggybackOutcome::Changed(next) => {
                            let next_cfg = *next;
                            if let Err(e) = crate::config::save(&next_cfg) {
                                tracing::warn!(error = %e, "persist launch-pulled config failed");
                            }
                            let _ = app_handle.emit("config-changed", &next_cfg);
                        }
                        crate::sync::PiggybackOutcome::Revoked => {
                            let mut reverted = cfg.clone();
                            reverted.sync_with_cloud = false;
                            if let Err(e) = crate::config::save(&reverted) {
                                tracing::warn!(error = %e, "persist launch-revoked config failed");
                            }
                            let _ = app_handle.emit("sync-revoked", ());
                            let _ = app_handle.emit("config-changed", &reverted);
                        }
                        _ => {}
                    }
                });
            }

            // 7. Debounced preferences pull on main-window focus.
            //    Fires whenever the user brings the tray panel to the
            //    front. The 5-second debounce prevents a burst of pulls
            //    when the OS fires multiple Focused events in quick
            //    succession (e.g. during window restore). The first
            //    focus is always allowed (debouncer starts as None).
            {
                use std::sync::{Arc as StdArc, Mutex};
                let last_focus_pull: StdArc<Mutex<Option<std::time::Instant>>> =
                    StdArc::new(Mutex::new(None));

                if let Some(window) = app.get_webview_window("main") {
                    let app_handle_focus = app.handle().clone();
                    let last_focus_pull_clone = last_focus_pull.clone();
                    window.on_window_event(move |event| {
                        if let tauri::WindowEvent::Focused(true) = event {
                            // Check debounce.
                            let mut guard = last_focus_pull_clone.lock().unwrap();
                            if !crate::sync::should_pull_on_focus(&mut guard) {
                                return;
                            }
                            drop(guard);

                            // Fire the pull async. Always reloads config
                            // from disk — the bulk-tick lane may have
                            // written new values since the closure was
                            // created.
                            let app_handle = app_handle_focus.clone();
                            tauri::async_runtime::spawn(async move {
                                let cfg = match crate::config::load() {
                                    Ok(c) => c,
                                    Err(_) => return,
                                };
                                match crate::sync::piggyback_preferences_pull(&cfg).await {
                                    crate::sync::PiggybackOutcome::Changed(next) => {
                                        let next_cfg = *next;
                                        if let Err(e) = crate::config::save(&next_cfg) {
                                            tracing::warn!(
                                                error = %e,
                                                "persist focus-pulled config failed"
                                            );
                                        }
                                        let _ = app_handle.emit("config-changed", &next_cfg);
                                    }
                                    crate::sync::PiggybackOutcome::Revoked => {
                                        let mut reverted = cfg.clone();
                                        reverted.sync_with_cloud = false;
                                        if let Err(e) = crate::config::save(&reverted) {
                                            tracing::warn!(
                                                error = %e,
                                                "persist focus-revoked config failed"
                                            );
                                        }
                                        let _ = app_handle.emit("sync-revoked", ());
                                        let _ = app_handle.emit("config-changed", &reverted);
                                    }
                                    _ => {}
                                }
                            });
                        }
                    });
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_config,
            commands::save_config,
            commands::set_sync_preset,
            commands::get_discovered_logs,
            commands::get_parse_coverage,
            commands::get_session_summary_text,
            commands::get_session_timeline,
            commands::search_events,
            commands::list_transactions,
            commands::get_app_version,
            commands::get_build_release_channel,
            commands::reparse_events,
            commands::reingest_rotated_logs,
            commands::check_for_update_for_channel,
            commands::install_update_for_channel,
            commands::get_source_stats,
            commands::get_storage_stats,
            commands::mark_event_as_noise,
            commands::pair_device,
            commands::refresh_account_info,
            commands::retry_sync_now,
            commands::get_sync_backlog,
            commands::check_upload_drift,
            commands::requeue_missing_events,
            commands::count_quarantined,
            commands::release_quarantined,
            commands::refresh_hangar_now,
            commands::set_rsi_cookie,
            commands::clear_rsi_cookie,
            commands::get_rsi_cookie_status,
            commands::set_org_bearer,
            commands::clear_org_bearer,
            commands::get_org_bearer_status,
            commands::get_health,
            commands::dismiss_health,
            commands::check_api_url,
            commands::check_rsi_cookie,
            commands::set_update_available,
            commands::list_unknown_lines,
            commands::count_unknown_lines,
            commands::dismiss_unknown_line,
            commands::submit_unknown_lines,
            commands::client_anon_id,
            commands::get_reference_category,
            commands::get_whats_new,
            commands::mark_whats_new_seen,
            commands::get_autostart_enabled,
            commands::set_autostart_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("tray icon".into()))?;

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("StarStats")
        .menu(&menu)
        // Left-click should not pop the menu — it should show the
        // window. Right-click still opens the menu (default platform
        // behavior).
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// True when the process was invoked by the OS autostart hook (the
/// `--autostart` arg is appended to the registered command in
/// `tauri_plugin_autostart::init`). Suppresses the first-launch main
/// window — the tray icon is the only visible affordance until the
/// user clicks it.
fn is_autostart_launch() -> bool {
    std::env::args().any(|a| a == "--autostart")
}

fn init_telemetry(debug_logging: bool) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,starstats=info"));
    let stdout_layer = fmt::layer().with_target(false);

    // The daily-rolling file appender is opt-in. With debug_logging
    // off (the default for end users) we keep the user's data dir
    // tidy — no log accumulation. Toggle from Settings → Updates.
    // Panic.log is still written on panic, regardless of this flag,
    // so we never lose a crash trace.
    if debug_logging {
        if let Ok(dir) = config::data_dir() {
            let file_appender = tracing_appender::rolling::daily(&dir, "client.log");
            let file_layer = fmt::layer()
                .with_writer(file_appender)
                .with_target(false)
                .with_ansi(false);
            let _ = Registry::default()
                .with(filter)
                .with(stdout_layer)
                .with(file_layer)
                .try_init();
            return;
        }
    }
    let _ = Registry::default()
        .with(filter)
        .with(stdout_layer)
        .try_init();
}

/// Capture panics to a dedicated `panic.log` in the user data dir
/// using direct unbuffered writes — a panic during setup can exit
/// the process within milliseconds, faster than tracing's pipeline
/// can flush. The default panic hook still runs afterwards so debug
/// builds keep the standard stderr trace.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic: {info}\nbacktrace:\n{backtrace}");

        if let Ok(dir) = config::data_dir() {
            let path = dir.join("panic.log");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                use std::io::Write;
                let _ = writeln!(
                    f,
                    "[{}] [v{}] panic: {info}\nbacktrace:\n{backtrace}\n---",
                    chrono::Utc::now().to_rfc3339(),
                    env!("CARGO_PKG_VERSION"),
                );
            }
        }

        default_hook(info);
    }));
}

/// Spawns the background sync worker (no-op if `remote_sync.enabled`
/// is false), fires a one-shot `/v1/auth/me` hydration, and spawns
/// the hangar refresh worker — all fire-and-forget. The hangar worker
/// is gated on api_url + access_token being present (no point pushing
/// to a server we can't authenticate against); per-cycle decisions
/// (cookie present? game running?) are made inside the worker itself.
/// The UI shows a neutral account state until the hydration lands,
/// and the user can trigger a manual refresh via the
/// `refresh_account_info` command.
#[allow(clippy::too_many_arguments)]
fn start_sync_workers(
    storage: Arc<Storage>,
    sync_stats: Arc<parking_lot::Mutex<sync::SyncStats>>,
    hangar_stats: Arc<parking_lot::Mutex<HangarStats>>,
    account_status: Arc<parking_lot::Mutex<AccountStatus>>,
    sync_kick: Arc<sync::SyncKick>,
    hangar_kick: Arc<tokio::sync::Notify>,
    app_handle: tauri::AppHandle,
    location_catalog: Arc<
        parking_lot::RwLock<Arc<starstats_core::location_catalog::LocationCatalog>>,
    >,
) -> sync::SyncHandles {
    let app_config = config::load().unwrap_or_default();

    let sync_handle = sync::start(
        app_config.remote_sync.clone(),
        app_config.sync_with_cloud,
        storage,
        sync_stats,
        Arc::clone(&account_status),
        sync_kick,
        app_handle,
        location_catalog,
    );

    if let (Some(api_url), Some(token)) = (
        app_config.remote_sync.api_url.clone(),
        app_config.remote_sync.access_token.clone(),
    ) {
        // Hangar refresh worker — same auth posture as sync (needs
        // the device token + api_url). Skips per-cycle if the user
        // hasn't pasted an RSI cookie yet, or if the game is running.
        // Fire-and-forget — the JoinHandle drops with the runtime.
        let _hangar_handle = hangar::start(
            api_url.clone(),
            token.clone(),
            Arc::clone(&hangar_stats),
            Arc::clone(&account_status),
            Arc::clone(&hangar_kick),
        );

        let account_status_for_init = Arc::clone(&account_status);
        tauri::async_runtime::spawn(async move {
            match sync::fetch_me(&api_url, &token).await {
                Ok(Some(me)) => {
                    let mut s = account_status_for_init.lock();
                    s.email_verified = Some(me.email_verified);
                    s.auth_lost = false;
                }
                Ok(None) => {
                    // Token rejected at startup — flip the banner so
                    // the user re-pairs before the next launch session.
                    tracing::warn!("startup /v1/auth/me rejected token — marking auth_lost");
                    let mut s = account_status_for_init.lock();
                    s.auth_lost = true;
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "startup /v1/auth/me failed — leaving account state neutral"
                    );
                }
            }
        });
    }

    sync_handle
}

/// Picks the largest discovered live `Game.log` and starts tailing
/// it. Returns `Ok(None)` when no candidate is found in any standard
/// install path — the tray still launches in that case so the user
/// can pair a device or change the configured path.
///
/// Only `LogKind::ChannelLive` entries are considered. Discovery now
/// also surfaces archived rotated logs and crash reports for UI
/// visibility, but those aren't tail-able sources — picking one
/// would mean reading a stale file with no ongoing updates.
fn start_log_tail(
    storage: Arc<Storage>,
    tail_stats: Arc<parking_lot::Mutex<gamelog::TailStats>>,
    rules: parser_defs::RuleCache,
    enable_v2_metadata: bool,
    own_handle: String,
    event_kick: Arc<tokio::sync::Notify>,
) -> anyhow::Result<Option<notify::RecommendedWatcher>> {
    let mut discovered: Vec<discovery::DiscoveredLog> = discovery::discover()
        .into_iter()
        .filter(|d| d.kind == discovery::LogKind::ChannelLive)
        .collect();
    discovered.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

    let Some(log) = discovered.first().cloned() else {
        tracing::warn!("no live Game.log discovered in standard install paths");
        return Ok(None);
    };

    tracing::info!(
        channel = %log.channel,
        path = %log.path.display(),
        "starting tail"
    );
    {
        let mut s = tail_stats.lock();
        s.current_path = Some(log.path.clone());
    }
    let path = log.path.clone();
    let watcher = tauri::async_runtime::block_on(async move {
        gamelog::start_tail(
            path,
            storage,
            tail_stats,
            rules,
            enable_v2_metadata,
            own_handle,
            event_kick,
        )
        .await
    })?;
    Ok(Some(watcher))
}
