//! Locate Star Citizen log artifacts on disk.
//!
//! Each Star Citizen install has one or more channel directories
//! (`LIVE/`, `PTU/`, `EPTU/`, `HOTFIX/`, `TECH-PREVIEW/`). We walk
//! each one and surface every artifact worth knowing about so the
//! user can pick (or so the tail orchestrator can fan out to all of
//! them in a future wave). Today the `start_log_tail` consumer only
//! cares about [`LogKind::ChannelLive`]; the other kinds are surfaced
//! to the UI as informational and as a discovery seed for follow-up
//! ingest paths (rotated-log backfill, crash-event signal).
//!
//! ## What we discover
//!
//! | Kind                    | Path shape                                              |
//! |-------------------------|---------------------------------------------------------|
//! | `ChannelLive`           | `<install>/<channel>/Game.log` (the running session)    |
//! | `ChannelArchived`       | `<install>/<channel>/Logs/Game-*.log` (rotated)         |
//! | `CrashReport`           | `<install>/<channel>/Crashes/<dir>/<file>.log` (crashes)|
//! | `LauncherLog`           | `%LOCALAPPDATA%/rsilauncher/logs/*.log` (RSI launcher)  |
//!
//! `ChannelLive` is the only kind the tail layer touches today. The
//! rest are surfaced so the UI can show "we see N rotated logs and M
//! crash dumps" — turning them into ingest sources is a future wave.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What kind of log artifact a [`DiscoveredLog`] points at. The tail
/// layer treats these very differently — `ChannelLive` is watched for
/// appended bytes; `ChannelArchived` would be read once if we wired
/// backfill; crash reports are signal-only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    /// The currently-active `Game.log` for a channel.
    ChannelLive,
    /// A rotated `Game-*.log` archived by the engine on launch.
    ChannelArchived,
    /// A `.log` file inside a `Crashes/<timestamp>/` directory.
    CrashReport,
    /// RSI Launcher's own log (`%LOCALAPPDATA%/rsilauncher/logs/`).
    LauncherLog,
}

/// One discovered log on disk. `channel` is `LIVE`/`PTU`/etc. for
/// channel-scoped artifacts and `LAUNCHER` for launcher logs (which
/// don't belong to a game channel).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveredLog {
    pub channel: String,
    pub kind: LogKind,
    pub path: PathBuf,
    pub size_bytes: u64,
}

const CHANNELS: &[&str] = &["LIVE", "PTU", "EPTU", "HOTFIX", "TECH-PREVIEW"];
const LAUNCHER_CHANNEL: &str = "LAUNCHER";

#[cfg(target_os = "windows")]
fn install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for drive in ['C', 'D', 'E', 'F'] {
        roots.push(PathBuf::from(format!(
            r"{drive}:\Program Files\Roberts Space Industries\StarCitizen"
        )));
    }
    roots
}

#[cfg(target_os = "linux")]
fn install_roots() -> Vec<PathBuf> {
    // Proton compatdata layouts vary by Steam/launcher; try the common
    // base paths and let the discovery loop probe each.
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        // Common Steam compatdata roots:
        roots.push(home.join(".steam/steam/steamapps/compatdata"));
        roots.push(home.join(".local/share/Steam/steamapps/compatdata"));
    }
    roots
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn install_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// Standard install roots for the RSI Launcher's own logs. The
/// launcher writes to `%LOCALAPPDATA%/rsilauncher/logs/` on Windows
/// and a Wine-prefix equivalent on Linux. Empty on macOS for now —
/// nobody runs SC there natively.
#[cfg(target_os = "windows")]
fn launcher_log_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("rsilauncher").join("logs"));
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(roaming).join("rsilauncher").join("logs"));
    }
    roots
}

#[cfg(not(target_os = "windows"))]
fn launcher_log_roots() -> Vec<PathBuf> {
    // On Linux the launcher runs under Proton and its logs live inside
    // the prefix; covering that requires walking compatdata in the
    // same way as the game install. Skip for now — most Linux users
    // run the game directly; launcher logs are a Windows-first concern.
    Vec::new()
}

fn meta_or_skip(path: &Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_file() {
        Some(meta.len())
    } else {
        None
    }
}

/// Walk a single rotated-logs directory (`Logs/` or `logbackups/`)
/// and emit a [`DiscoveredLog::ChannelArchived`] for each file that
/// looks like a rotated Game.log. Filename matcher is permissive —
/// "starts with Game, ends with .log, isn't the live Game.log" —
/// so we cover the historical `Game-YYYYMMDD-HHMMSS.log` form, the
/// 2025+ logbackups naming, and copies/moves users sometimes do.
/// Unrelated `.log` files (CrashGame, internal traces) are still
/// excluded by the `Game` prefix.
fn collect_rotated_from(channel: &str, dir: &Path, out: &mut Vec<DiscoveredLog>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("Game") || !name.ends_with(".log") || name == "Game.log" {
            continue;
        }
        let Some(size) = meta_or_skip(&path) else {
            continue;
        };
        out.push(DiscoveredLog {
            channel: channel.to_string(),
            kind: LogKind::ChannelArchived,
            path,
            size_bytes: size,
        });
    }
}

/// Walk a single channel directory and emit every discovered log
/// artifact. `channel` is the directory name (e.g. `LIVE`); `channel_dir`
/// is the absolute path to it.
fn collect_channel(channel: &str, channel_dir: &Path, out: &mut Vec<DiscoveredLog>) {
    // 1. Live Game.log — the only file the tail layer currently
    //    consumes. Always probed first so it appears at the top of
    //    a stable-ordered output.
    let live = channel_dir.join("Game.log");
    if let Some(size) = meta_or_skip(&live) {
        out.push(DiscoveredLog {
            channel: channel.to_string(),
            kind: LogKind::ChannelLive,
            path: live,
            size_bytes: size,
        });
    }

    // 2. Rotated logs. The engine archives historical Game.log
    //    sessions to one of two sibling directories depending on
    //    install/era:
    //      - `<channel>/Logs/`        (older naming, `Game-YYYY-...log`)
    //      - `<channel>/logbackups/`  (current 2025+ naming)
    //    Both are walked with the same filename filter — we don't
    //    care which one a given install uses, and copying logs between
    //    the two is a real thing users do.
    let mut rotated: Vec<DiscoveredLog> = Vec::new();
    for sub in &["Logs", "logbackups"] {
        collect_rotated_from(channel, &channel_dir.join(sub), &mut rotated);
    }
    // Newest first by filename — the timestamp embedded in the name
    // sorts correctly as a string. The UI cares about "what did the
    // most recent session look like" more than alphabetical order.
    rotated.sort_by(|a, b| b.path.file_name().cmp(&a.path.file_name()));
    out.append(&mut rotated);

    // 3. Crash reports. Each crash drops a directory under Crashes/
    //    containing a minidump and one or more text logs. We surface
    //    the most informative `.log` file from each crash dir so a
    //    future wave can parse stack traces / engine version off the
    //    top. Skip the .dmp binaries — they're for human triage in a
    //    debugger, not for our event store.
    let crashes_dir = channel_dir.join("Crashes");
    if let Ok(entries) = std::fs::read_dir(&crashes_dir) {
        let mut crashes: Vec<DiscoveredLog> = entries
            .flatten()
            .filter_map(|entry| {
                let crash_dir = entry.path();
                if !crash_dir.is_dir() {
                    return None;
                }
                // Pick the largest .log inside the crash dir — the
                // engine writes a short summary plus a longer detail
                // log; we surface the bigger one (more context).
                let mut best: Option<(PathBuf, u64)> = None;
                if let Ok(files) = std::fs::read_dir(&crash_dir) {
                    for f in files.flatten() {
                        let p = f.path();
                        if p.extension().and_then(|e| e.to_str()) != Some("log") {
                            continue;
                        }
                        let Some(size) = meta_or_skip(&p) else {
                            continue;
                        };
                        if best.as_ref().map_or(true, |(_, s)| size > *s) {
                            best = Some((p, size));
                        }
                    }
                }
                let (path, size) = best?;
                Some(DiscoveredLog {
                    channel: channel.to_string(),
                    kind: LogKind::CrashReport,
                    path,
                    size_bytes: size,
                })
            })
            .collect();
        crashes.sort_by(|a, b| b.path.file_name().cmp(&a.path.file_name()));
        out.append(&mut crashes);
    }
}

fn collect_from_root(root: &Path, out: &mut Vec<DiscoveredLog>) {
    for channel in CHANNELS {
        let channel_dir = root.join(channel);
        if !channel_dir.exists() {
            continue;
        }
        collect_channel(channel, &channel_dir, out);
    }
}

/// Walk the launcher log root and surface every `*.log` file. The
/// launcher rotates by date; we surface them all so the UI can show
/// "how chatty has the launcher been" and a future wave can parse
/// login / patch events off the top.
fn collect_launcher_logs(out: &mut Vec<DiscoveredLog>) {
    for root in launcher_log_roots() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut found: Vec<DiscoveredLog> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("log") {
                    return None;
                }
                let size = meta_or_skip(&path)?;
                Some(DiscoveredLog {
                    channel: LAUNCHER_CHANNEL.to_string(),
                    kind: LogKind::LauncherLog,
                    path,
                    size_bytes: size,
                })
            })
            .collect();
        found.sort_by(|a, b| b.path.file_name().cmp(&a.path.file_name()));
        out.append(&mut found);
    }
}

pub fn discover() -> Vec<DiscoveredLog> {
    let mut out = Vec::new();
    for root in install_roots() {
        if !root.exists() {
            continue;
        }
        // Direct Windows-style: <root>/<channel>/...
        collect_from_root(&root, &mut out);

        // Linux Proton: <compatdata>/<id>/pfx/drive_c/Program Files/Roberts Space Industries/StarCitizen/<channel>/...
        if cfg!(target_os = "linux") {
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let inner = entry
                        .path()
                        .join("pfx/drive_c/Program Files/Roberts Space Industries/StarCitizen");
                    if inner.exists() {
                        collect_from_root(&inner, &mut out);
                    }
                }
            }
        }
    }
    collect_launcher_logs(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    /// Builds a synthetic install layout under `root` matching the
    /// shapes documented in the module header. Returns the root for
    /// the channel walker to consume.
    fn build_channel(root: &Path, channel: &str) -> PathBuf {
        let dir = root.join(channel);
        fs::create_dir_all(&dir).unwrap();
        write_file(&dir.join("Game.log"), b"<2026-01-01> live\n");
        let logs = dir.join("Logs");
        fs::create_dir_all(&logs).unwrap();
        write_file(
            &logs.join("Game-20260101-120000.log"),
            b"<2026-01-01> archived\n",
        );
        write_file(
            &logs.join("Game-20260102-120000.log"),
            b"<2026-01-02> archived\n",
        );
        // Unrelated file the engine sometimes drops in Logs/ — must
        // NOT be surfaced as ChannelArchived.
        write_file(&logs.join("CrashGame.log"), b"unrelated");
        let crashes = dir.join("Crashes");
        let crash_a = crashes.join("2026-01-01-crash");
        fs::create_dir_all(&crash_a).unwrap();
        // Two .log files in the crash dir — the larger should win.
        write_file(&crash_a.join("crash-summary.log"), b"summary");
        write_file(&crash_a.join("crash-detail.log"), &vec![b'D'; 5_000]);
        write_file(&crash_a.join("crash.dmp"), b"binary minidump");
        dir
    }

    fn write_file(path: &Path, body: &[u8]) {
        let mut f = fs::File::create(path).unwrap();
        f.write_all(body).unwrap();
    }

    #[test]
    fn collect_channel_emits_live_archived_and_crash_entries() {
        let tmp = TempDir::new().unwrap();
        let channel_dir = build_channel(tmp.path(), "LIVE");
        let mut out = Vec::new();
        collect_channel("LIVE", &channel_dir, &mut out);

        // 1 live + 2 archived + 1 crash report (the largest .log in
        // the single crash dir) = 4 entries.
        assert_eq!(out.len(), 4, "got {out:#?}");

        // Live entry first.
        assert_eq!(out[0].kind, LogKind::ChannelLive);
        assert!(out[0].path.ends_with("Game.log"));

        // Archived entries are newest-first by filename.
        assert_eq!(out[1].kind, LogKind::ChannelArchived);
        assert!(out[1]
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("20260102"));
        assert_eq!(out[2].kind, LogKind::ChannelArchived);
        assert!(out[2]
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("20260101"));

        // Crash entry: must point at the larger of the two .log files
        // and never at the .dmp.
        assert_eq!(out[3].kind, LogKind::CrashReport);
        assert!(out[3]
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("crash-detail.log"));
    }

    #[test]
    fn collect_channel_picks_up_logbackups_directory() {
        // 2025+ rotation puts archives in `<channel>/logbackups/`
        // instead of `Logs/`. The walker must check both. Filename
        // matcher is permissive (`Game*.log`) to cover both the
        // historical `Game-YYYYMMDD-...log` and any newer naming.
        let tmp = TempDir::new().unwrap();
        let channel_dir = tmp.path().join("LIVE");
        fs::create_dir_all(&channel_dir).unwrap();
        write_file(&channel_dir.join("Game.log"), b"live\n");
        let backups = channel_dir.join("logbackups");
        fs::create_dir_all(&backups).unwrap();
        write_file(&backups.join("Game.20260103.log"), b"<2026-01-03>\n");
        write_file(&backups.join("Game.20260104.log"), b"<2026-01-04>\n");
        // Unrelated — must NOT be surfaced.
        write_file(&backups.join("CrashGame.log"), b"unrelated");
        write_file(&backups.join("readme.txt"), b"unrelated");

        let mut out = Vec::new();
        collect_channel("LIVE", &channel_dir, &mut out);

        // 1 live + 2 from logbackups = 3 entries; prove the two
        // logbackups files came through.
        let archived: Vec<_> = out
            .iter()
            .filter(|e| e.kind == LogKind::ChannelArchived)
            .collect();
        assert_eq!(archived.len(), 2, "got {out:#?}");
        // Newest-first by filename.
        assert!(archived[0]
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("20260104"));
        assert!(archived[1]
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("20260103"));
    }

    #[test]
    fn collect_channel_skips_non_game_log_files_in_logs_dir() {
        let tmp = TempDir::new().unwrap();
        let channel_dir = build_channel(tmp.path(), "PTU");
        let mut out = Vec::new();
        collect_channel("PTU", &channel_dir, &mut out);
        // The `CrashGame.log` we wrote into Logs/ must not appear as
        // a ChannelArchived row — the prefix filter excludes it.
        for entry in &out {
            if entry.kind == LogKind::ChannelArchived {
                let name = entry.path.file_name().unwrap().to_string_lossy();
                assert!(
                    name.starts_with("Game-"),
                    "non-Game- file slipped in: {name}"
                );
            }
        }
    }

    #[test]
    fn collect_channel_handles_missing_subdirs() {
        // No Logs/, no Crashes/, only a bare Game.log — must still
        // produce one row, not panic.
        let tmp = TempDir::new().unwrap();
        let channel_dir = tmp.path().join("LIVE");
        fs::create_dir_all(&channel_dir).unwrap();
        write_file(&channel_dir.join("Game.log"), b"x");
        let mut out = Vec::new();
        collect_channel("LIVE", &channel_dir, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, LogKind::ChannelLive);
    }

    #[test]
    fn collect_channel_emits_nothing_when_channel_dir_is_empty() {
        let tmp = TempDir::new().unwrap();
        let channel_dir = tmp.path().join("EPTU");
        fs::create_dir_all(&channel_dir).unwrap();
        let mut out = Vec::new();
        collect_channel("EPTU", &channel_dir, &mut out);
        assert!(out.is_empty(), "empty channel dir should yield nothing");
    }

    #[test]
    fn collect_channel_skips_crash_dir_with_no_log_files() {
        // A crash dir that only contains a .dmp must be skipped — we
        // surface readable logs, not minidumps.
        let tmp = TempDir::new().unwrap();
        let channel_dir = tmp.path().join("LIVE");
        fs::create_dir_all(channel_dir.join("Crashes/dump-only")).unwrap();
        write_file(&channel_dir.join("Crashes/dump-only/crash.dmp"), b"binary");
        let mut out = Vec::new();
        collect_channel("LIVE", &channel_dir, &mut out);
        assert!(
            out.iter().all(|e| e.kind != LogKind::CrashReport),
            "dump-only crash dir leaked into output"
        );
    }
}
