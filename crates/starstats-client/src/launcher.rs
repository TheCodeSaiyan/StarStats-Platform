//! RSI Launcher log tailer.
//!
//! Mirrors the shape of [`crate::gamelog::start_tail`] but parses a
//! different format: the launcher writes Electron-style entries
//!
//!   `[2026-05-06 12:34:56.789] [info] message text`
//!
//! Each recognised line becomes a [`LauncherActivity`] event. We
//! deliberately don't classify message contents yet — there's no
//! stable vocabulary the way `<Init>` / `<Join PU>` give us in
//! `Game.log`. Once we have a body of sample lines, follow-up waves
//! can layer specific variants (login, install, patch progress)
//! over the same tail.
//!
//! The launcher rotates by date, so this module picks the
//! lexicographically-newest log on startup. A future wave could
//! re-pick on a periodic tick if the user keeps the tray running
//! across midnight; today the cost of missing the rotation is "you
//! see new launcher events after your next tray restart" which is a
//! fine trade.

use crate::discovery::{self, LogKind};
use crate::gamelog::{file_signature, idempotency_key, resolve_resume_offset};
use crate::storage::Storage;
use anyhow::Result;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use starstats_core::{classify_launcher_message, parse_launcher_line, GameEvent, LauncherActivity};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader, SeekFrom};
use tokio::sync::mpsc;

/// Live counters surfaced to the frontend so the user can see the
/// launcher tail is working without inspecting individual events.
#[derive(Debug, Default, Clone, Serialize)]
pub struct LauncherStats {
    pub current_path: Option<PathBuf>,
    pub bytes_read: u64,
    pub lines_processed: u64,
    pub events_recognised: u64,
    pub last_event_at: Option<String>,
    pub last_level: Option<String>,
    /// Category of the most-recent classified message, snake_case.
    /// Surfaced to the UI so the user can see "the launcher is busy
    /// patching" without opening the timeline.
    pub last_category: Option<String>,
    pub lines_skipped: u64,
}

/// Discover the most-recent launcher log and start tailing it.
/// Returns `Ok(None)` when no launcher logs are found in any
/// standard install path — the caller treats this as "user doesn't
/// have the RSI Launcher installed locally" and skips the tail
/// silently. Returns the watcher handle on success — drop it to stop.
pub async fn start_tail(
    storage: Arc<Storage>,
    stats: Arc<parking_lot::Mutex<LauncherStats>>,
) -> Result<Option<RecommendedWatcher>> {
    let mut launcher_logs: Vec<_> = discovery::discover()
        .into_iter()
        .filter(|d| d.kind == LogKind::LauncherLog)
        .collect();

    // Discovery sorts launcher logs newest-first by filename, so [0]
    // is the file most likely to be receiving live appends. We commit
    // to that one for the lifetime of the tray.
    let Some(target) = launcher_logs.drain(..).next() else {
        tracing::info!("no RSI Launcher logs discovered; launcher tail not started");
        return Ok(None);
    };

    let path = target.path.clone();
    tracing::info!(path = %path.display(), "starting RSI Launcher tail");
    {
        let mut s = stats.lock();
        s.current_path = Some(path.clone());
    }

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
    let (mut offset, mut last_sig) = storage.read_tail_cursor(&path_str)?;
    let path_clone = path.clone();
    let path_str_clone = path_str.clone();
    let storage_clone = Arc::clone(&storage);
    let stats_clone = Arc::clone(&stats);

    tokio::spawn(async move {
        // Initial drain — pick up everything since the last cursor
        // position, including the lines that landed while the tray
        // was offline.
        if let Err(e) = drain(
            &path_clone,
            &path_str_clone,
            &mut offset,
            &mut last_sig,
            &storage_clone,
            &stats_clone,
        )
        .await
        {
            tracing::warn!(error = %e, "initial launcher drain failed");
        }

        while rx.recv().await.is_some() {
            // Coalesce bursts of filesystem events.
            while rx.try_recv().is_ok() {}
            if let Err(e) = drain(
                &path_clone,
                &path_str_clone,
                &mut offset,
                &mut last_sig,
                &storage_clone,
                &stats_clone,
            )
            .await
            {
                tracing::warn!(error = %e, "launcher drain failed; backing off");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });

    Ok(Some(watcher))
}

async fn drain(
    path: &PathBuf,
    path_str: &str,
    offset: &mut u64,
    last_sig: &mut Option<String>,
    storage: &Storage,
    stats: &parking_lot::Mutex<LauncherStats>,
) -> Result<()> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    let metadata = file.metadata().await?;

    // Detect rotation via the file's head signature, not just a length
    // shrink — a replacement log already longer than our saved offset
    // would otherwise be seeked mid-file. Shared with the Game.log
    // tail; see `crate::gamelog::resolve_resume_offset`.
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
            "launcher tail resume offset reset (rotation/truncation)",
        );
    }
    *offset = resumed;
    *last_sig = current_sig;

    if metadata.len() == *offset {
        return Ok(());
    }

    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(*offset)).await?;
    let mut buf = String::new();

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
        process_line(
            buf.trim_end_matches(['\r', '\n']),
            storage,
            stats,
            line_start,
            last_sig.as_deref(),
        );
        let mut s = stats.lock();
        s.bytes_read = *offset;
        s.lines_processed += 1;
    }

    storage.write_tail_cursor(path_str, *offset, last_sig.as_deref())?;
    Ok(())
}

fn process_line(
    line: &str,
    storage: &Storage,
    stats: &parking_lot::Mutex<LauncherStats>,
    line_offset: u64,
    file_sig: Option<&str>,
) {
    let Some(parsed) = parse_launcher_line(line) else {
        // Banners, blanks, multi-line continuations — not actionable.
        stats.lock().lines_skipped += 1;
        return;
    };
    let level = parsed.level.to_ascii_lowercase();
    let category = classify_launcher_message(&level, parsed.message);
    let event = GameEvent::LauncherActivity(LauncherActivity {
        timestamp: parsed.timestamp.to_string(),
        level,
        message: parsed.message.to_string(),
        category,
    });
    let payload = match serde_json::to_string(&event) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "launcher event serialise failed");
            return;
        }
    };
    let idempotency_key = idempotency_key("launcher", file_sig, line_offset, line);
    if let Err(e) = storage.insert_event(
        &idempotency_key,
        "launcher_activity",
        parsed.timestamp,
        line,
        &payload,
        // Distinct log_source so launcher events don't get confused
        // with channel-specific gameplay events in the read API.
        "launcher",
        line_offset,
    ) {
        tracing::warn!(error = %e, "launcher insert_event failed");
        return;
    }
    let mut s = stats.lock();
    s.events_recognised += 1;
    s.last_event_at = Some(parsed.timestamp.to_string());
    s.last_level = Some(parsed.level.to_ascii_lowercase());
    s.last_category = Some(launcher_category_str(category).to_string());
}

/// Snake-case form of [`LauncherCategory`]. Matches what serde
/// produces; surfaced to the UI as a stat row.
fn launcher_category_str(c: starstats_core::LauncherCategory) -> &'static str {
    use starstats_core::LauncherCategory::*;
    match c {
        Auth => "auth",
        Install => "install",
        Patch => "patch",
        Update => "update",
        Error => "error",
        Info => "info",
    }
}
