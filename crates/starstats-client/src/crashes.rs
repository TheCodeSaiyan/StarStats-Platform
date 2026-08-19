//! Crash-dump scanner.
//!
//! Star Citizen drops a directory under `<install>/<channel>/Crashes/`
//! every time the engine crashes hard enough to flush a minidump. The
//! directory is named with an ISO-ish timestamp (e.g.
//! `2026-05-04-21-10-12`) and contains a `.dmp` plus one or more
//! `.log` files.
//!
//! We don't parse the dump body — the **fact of a crash** is the
//! signal worth surfacing. This module:
//!
//! 1. Walks every channel's `Crashes/` directory at startup and on a
//!    slow periodic poll.
//! 2. Synthesises one [`GameCrash`] event per directory.
//! 3. Uses the directory name as part of the idempotency key so a
//!    re-scan never produces duplicates, even across client restarts.
//!
//! The watcher cadence is intentionally low (60s) because crash dirs
//! are stable on disk — once one appears it isn't going anywhere, and
//! the cost of being a few seconds late on the timeline is zero.

use crate::discovery::{self, DiscoveredLog, LogKind};
use crate::storage::Storage;
use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;
use starstats_core::{GameCrash, GameEvent};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// How often the background scanner walks the install tree. Crash
/// dirs are stable once written, so a slow cadence is enough — the
/// initial sweep at startup catches the long-tail historical crashes;
/// the periodic sweep catches new crashes from sessions started after
/// the tray launched.
pub const SCAN_INTERVAL: Duration = Duration::from_secs(60);

/// Stats surfaced to the frontend so the user can confirm the
/// scanner is doing something even when no new crashes are found.
#[derive(Debug, Default, Clone, Serialize)]
pub struct CrashStats {
    pub last_scan_at: Option<String>,
    pub total_crashes_seen: u64,
    pub last_crash_dir: Option<String>,
}

/// Spawn the background scanner. Runs an immediate sweep then loops
/// at [`SCAN_INTERVAL`]. The handle to the scanner task is dropped
/// here — the loop runs for the lifetime of the process.
///
/// Uses `tauri::async_runtime::spawn` rather than `tokio::spawn`:
/// the Tauri 2 setup closure runs synchronously on the main thread
/// without a tokio runtime in TLS, so a raw `tokio::spawn` panics
/// with "no reactor running". Tauri's wrapper queues onto the
/// runtime it owns.
pub fn spawn_scanner(storage: Arc<Storage>, stats: Arc<parking_lot::Mutex<CrashStats>>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = sweep_once(&storage, &stats).await {
                tracing::warn!(error = %e, "crash scanner sweep failed");
            }
            tokio::time::sleep(SCAN_INTERVAL).await;
        }
    });
}

async fn sweep_once(storage: &Storage, stats: &parking_lot::Mutex<CrashStats>) -> Result<()> {
    // Discovery already does the filesystem walk for us — every
    // CrashReport-kind entry corresponds to one crash dir's primary
    // log. Reuse that path so we don't duplicate the walking logic.
    let discovered = discovery::discover();
    let crash_logs: Vec<DiscoveredLog> = discovered
        .into_iter()
        .filter(|d| d.kind == LogKind::CrashReport)
        .collect();

    for log in &crash_logs {
        if let Err(e) = ingest_crash(storage, log) {
            tracing::warn!(
                path = %log.path.display(),
                error = %e,
                "ingest_crash failed",
            );
        }
    }

    // Discovery returns crashes newest-first per channel, so the
    // first entry is the most recent crash on disk. We surface that
    // as `last_crash_dir` regardless of whether THIS sweep actually
    // inserted it — `insert_event` is idempotent on the dir name, so
    // the user-visible "your last crash was X" answer doesn't depend
    // on the sweep that found it.
    let mut s = stats.lock();
    s.last_scan_at = Some(Utc::now().to_rfc3339());
    s.total_crashes_seen = crash_logs.len() as u64;
    if let Some(first) = crash_logs.first() {
        s.last_crash_dir = crash_dir_name(&first.path);
    }
    Ok(())
}

/// Insert one crash event. Idempotent on `(channel, dir_name)` — the
/// underlying `insert_event` ON CONFLICT swallows duplicates so a
/// re-scan of the same Crashes/ folder doesn't double-count.
fn ingest_crash(storage: &Storage, log: &DiscoveredLog) -> Result<()> {
    let crash_dir = log.path.parent().ok_or_else(|| {
        anyhow::anyhow!("crash log path has no parent dir: {}", log.path.display())
    })?;
    let dir_name = crash_dir
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("crash dir name not UTF-8: {}", crash_dir.display()))?;
    let timestamp = parse_crash_timestamp(&dir_name)
        .or_else(|| dir_mtime_rfc3339(crash_dir))
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let primary_log_name = log
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string);
    let total_size_bytes = sum_crash_dir_bytes(crash_dir);

    let event = GameEvent::GameCrash(GameCrash {
        timestamp: timestamp.clone(),
        channel: log.channel.clone(),
        crash_dir_name: dir_name.clone(),
        primary_log_name,
        total_size_bytes,
    });
    let payload = serde_json::to_string(&event)?;

    // Idempotency anchored to dir + channel — re-scans can't dupe and
    // the same crash dir copied to a different channel (rare but
    // possible during a parallel install) wouldn't collide either.
    let idempotency_key = format!("crash:{}:{}", log.channel, dir_name);

    let log_source = log_source_for_channel(&log.channel);
    storage.insert_event(
        &idempotency_key,
        "game_crash",
        &timestamp,
        &dir_name, // synthetic events have no source line; surface dir name
        &payload,
        &log_source,
        0,
    )?;
    Ok(())
}

/// Sum the byte sizes of every regular file inside `dir`. Errors are
/// swallowed (treated as 0) — this is a UI signal, not a
/// correctness-critical figure.
fn sum_crash_dir_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total = total.saturating_add(meta.len());
                }
            }
        }
    }
    total
}

/// Parse a crash dir name like `2026-05-04-21-10-12` into RFC3339.
/// Returns `None` for any name that doesn't fit the pattern; the
/// caller falls back to the dir's mtime.
fn parse_crash_timestamp(dir_name: &str) -> Option<String> {
    let nt = NaiveDateTime::parse_from_str(dir_name, "%Y-%m-%d-%H-%M-%S").ok()?;
    let dt: DateTime<Utc> = Utc.from_utc_datetime(&nt);
    Some(dt.to_rfc3339())
}

fn dir_mtime_rfc3339(dir: &Path) -> Option<String> {
    let meta = std::fs::metadata(dir).ok()?;
    let mtime = meta.modified().ok()?;
    let dt: DateTime<Utc> = mtime.into();
    Some(dt.to_rfc3339())
}

fn crash_dir_name(crash_log_path: &Path) -> Option<String> {
    crash_log_path
        .parent()?
        .file_name()?
        .to_str()
        .map(str::to_string)
}

/// Map a discovery `channel` (LIVE/PTU/EPTU/...) to the wire
/// `LogSource` discriminant. Mirrors the routine in `gamelog.rs` but
/// works off the channel string instead of a path; kept private here
/// to avoid a public-API exchange between the two modules for one
/// trivial mapping.
fn log_source_for_channel(channel: &str) -> String {
    match channel {
        "LIVE" => "live",
        "PTU" => "ptu",
        "EPTU" => "eptu",
        "HOTFIX" => "hotfix",
        "TECH-PREVIEW" => "tech",
        _ => "other",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_crash_timestamp_handles_iso_dash_separated() {
        let ts = parse_crash_timestamp("2026-05-04-21-10-12").unwrap();
        // Round-trip through chrono — exact rfc3339 string varies by
        // chrono version; assert the prefix is what we expect.
        assert!(ts.starts_with("2026-05-04T21:10:12"));
    }

    #[test]
    fn parse_crash_timestamp_returns_none_for_garbage() {
        assert!(parse_crash_timestamp("not-a-timestamp").is_none());
        assert!(parse_crash_timestamp("").is_none());
        // Off-by-one digit count — must NOT silently match.
        assert!(parse_crash_timestamp("2026-05-04").is_none());
    }

    #[test]
    fn log_source_for_channel_lowercases_known_channels() {
        assert_eq!(log_source_for_channel("LIVE"), "live");
        assert_eq!(log_source_for_channel("PTU"), "ptu");
        assert_eq!(log_source_for_channel("UNKNOWN"), "other");
    }
}
