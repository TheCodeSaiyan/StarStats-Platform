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
/// Channel assigned to an override-supplied log whose parent directory
/// isn't one of [`CHANNELS`] — the user pointed at a copied/renamed log.
const CUSTOM_CHANNEL: &str = "CUSTOM";

/// Relative layouts, under a Windows volume root or inside a Wine
/// prefix's `drive_c`, where a Star Citizen install has been seen.
/// The first is the RSI Launcher default; the rest are the shapes
/// users pick when they move the install off the system drive.
///
/// Written with forward slashes so one table serves both platforms;
/// [`join_shape`] splits them back into segments so the joined path
/// carries the host's own separator. Joining the whole string in one
/// go works, but leaves `C:\Program Files/Roberts Space
/// Industries/StarCitizen\LIVE` in every path the UI displays.
const INSTALL_SHAPES: &[&str] = &[
    "Program Files/Roberts Space Industries/StarCitizen",
    "Roberts Space Industries/StarCitizen",
    "Games/Roberts Space Industries/StarCitizen",
    "Games/StarCitizen",
    "StarCitizen",
];

/// Join a `/`-separated [`INSTALL_SHAPES`] entry onto `base` one
/// segment at a time, so the result uses the platform separator
/// throughout rather than whichever one each half was written with.
fn join_shape(base: &Path, shape: &str) -> PathBuf {
    shape
        .split('/')
        .fold(base.to_path_buf(), |acc, seg| acc.join(seg))
}

/// Volume roots to probe on Windows. Enumerated from the mounted
/// volumes rather than a hardcoded `C..F`, because the install lands
/// wherever the user pointed the launcher's library at.
///
/// We ask `sysinfo` for the mount points rather than walking `A..Z`
/// ourselves: probing a drive letter that maps to a disconnected
/// network share blocks for seconds per call, and a blind alphabet
/// sweep would hit every one of them at startup.
#[cfg(target_os = "windows")]
fn volume_roots() -> Vec<PathBuf> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut roots: Vec<PathBuf> = disks
        .iter()
        .map(|d| d.mount_point().to_path_buf())
        .collect();
    // If the volume enumeration comes back empty (permissions, an
    // unexpected sysinfo failure) fall back to the drives the old
    // hardcoded list covered so we never regress a working install.
    if roots.is_empty() {
        for drive in ['C', 'D', 'E', 'F'] {
            roots.push(PathBuf::from(format!(r"{drive}:\")));
        }
    }
    roots
}

#[cfg(target_os = "windows")]
fn install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for volume in volume_roots() {
        for shape in INSTALL_SHAPES {
            roots.push(join_shape(&volume, shape));
        }
    }
    // The RSI Launcher writes the library path it installs into to its
    // own log. That is the authoritative answer for a custom install —
    // the shape table above is only a guess.
    roots.extend(launcher_log_install_roots());
    dedupe(roots)
}

/// Directories that either ARE a Wine prefix or CONTAIN one per child
/// entry. Star Citizen on Linux runs under Lutris, Heroic, Bottles or
/// a hand-rolled prefix far more often than under Steam, so probing
/// Steam's `compatdata` alone finds nothing for most users.
///
/// Returned as `(path, scan_children)`: `scan_children` means the
/// directory holds one prefix per child (Lutris' `prefixes/`, Steam's
/// `compatdata/`), rather than being a prefix itself.
#[cfg(target_os = "linux")]
fn prefix_roots() -> Vec<(PathBuf, bool)> {
    let mut roots: Vec<(PathBuf, bool)> = Vec::new();

    // An explicitly configured prefix beats every guess below.
    if let Some(prefix) = std::env::var_os("WINEPREFIX") {
        roots.push((PathBuf::from(prefix), false));
    }

    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return roots;
    };

    // Single prefixes, in the places the common install guides put them.
    for direct in [
        "Games/star-citizen",
        "Games/star-citizen/StarCitizen",
        "Games/StarCitizen",
        ".wine",
    ] {
        roots.push((home.join(direct), false));
    }

    // Directories holding one prefix per child.
    for container in [
        // Lutris, native and Flatpak.
        ".local/share/lutris/prefixes",
        ".var/app/net.lutris.Lutris/data/lutris/prefixes",
        // Bottles, native and Flatpak.
        ".local/share/bottles/bottles",
        ".var/app/com.usebottles.bottles/data/bottles/bottles",
        // Heroic, native and Flatpak.
        "Games/Heroic/Prefixes/default",
        ".var/app/com.heroicgameslauncher.hgl/config/heroic/Prefixes/default",
        // Steam Proton — the RSI Launcher added as a non-Steam game.
        ".steam/steam/steamapps/compatdata",
        ".local/share/Steam/steamapps/compatdata",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/compatdata",
        // A bare "put all my prefixes here" directory.
        "Games",
    ] {
        roots.push((home.join(container), true));
    }

    roots
}

/// Expand one prefix directory into the install roots it could hold.
/// A Steam `compatdata` entry nests the prefix under `pfx/`; every
/// other launcher uses the prefix directory itself. Both are probed
/// because the cost is an `exists()` call.
#[cfg(target_os = "linux")]
fn install_roots_in_prefix(prefix: &Path, out: &mut Vec<PathBuf>) {
    for drive_c in [prefix.join("drive_c"), prefix.join("pfx").join("drive_c")] {
        for shape in INSTALL_SHAPES {
            out.push(join_shape(&drive_c, shape));
        }
    }
}

#[cfg(target_os = "linux")]
fn install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for (root, scan_children) in prefix_roots() {
        install_roots_in_prefix(&root, &mut roots);
        if !scan_children {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            install_roots_in_prefix(&entry.path(), &mut roots);
        }
    }
    // Some users bind-mount or symlink the install outside any prefix.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for shape in INSTALL_SHAPES {
            roots.push(join_shape(&home, shape));
        }
    }
    dedupe(roots)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn install_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// Drop duplicate roots while keeping first-seen order. The shape
/// table and the launcher-log seed overlap on a default install, and
/// a duplicate root would emit every log under it twice.
fn dedupe(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    roots
        .into_iter()
        .filter(|r| seen.insert(r.clone()))
        .collect()
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

/// Pull every install root the RSI Launcher mentions out of one of
/// its log lines.
///
/// The launcher logs its library path verbatim on install, update,
/// verify and disk-space phases, in lines like:
///
/// ```text
/// [Pipeline] Installing Star Citizen LIVE 4.10.0 at C:\Games\Roberts Space Industries\StarCitizen (type: update, …)
/// [ComputeSizePhase] Computing required space for (SC LIVE) in C:\Games\Roberts Space Industries\StarCitizen\LIVE
/// ```
///
/// This is the only *authoritative* signal for a custom install — the
/// shape table in [`INSTALL_SHAPES`] is guesswork by comparison. We
/// scan for the literal `StarCitizen` segment and walk left to the
/// start of the absolute path, cutting at `StarCitizen` so both line
/// shapes above collapse to the same install root regardless of the
/// channel segment the second one appends.
///
/// Deliberately permissive about what surrounds the path: the log is
/// JSON-ish with escaped backslashes in some lines and raw ones in
/// others, and the surrounding text changes between launcher versions.
/// Anything that doesn't resolve to a real directory is dropped by the
/// caller's `exists()` probe anyway.
/// Compiled on every platform because its tests are, but only the
/// Windows scanner above consumes it: on Linux the launcher's logs sit
/// inside the Wine prefix we are still trying to locate, so there is no
/// bootstrap and no caller. Without the `allow`, `cargo check` on Linux
/// fails the workspace's `-D warnings` gate.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn install_roots_in_log_line(line: &str) -> Vec<PathBuf> {
    const MARKER: &str = r"StarCitizen";
    let mut out = Vec::new();

    for (idx, _) in line.match_indices(MARKER) {
        // The match has to be a whole path SEGMENT. Without this,
        // `…\StarCitizen\LIVE\StarCitizen.exe` yields a second, bogus
        // root ending at the executable's name — observed on a real
        // launcher log, harmless only because it fails `exists()`.
        let after = line[idx + MARKER.len()..].chars().next();
        if matches!(after, Some(c) if c.is_alphanumeric() || c == '.' || c == '_' || c == '-') {
            continue;
        }
        let head = &line[..idx];
        // Walk left to the drive letter that opens this absolute path.
        // `X:` preceded by a separator or start-of-field.
        let Some(colon) = head.rfind(':') else {
            continue;
        };
        if colon == 0 {
            continue;
        }
        let drive_start = colon - 1;
        if !head[drive_start..colon].starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let raw = &line[drive_start..idx + MARKER.len()];
        // The log escapes separators in some fields (`C:\\Games\\…`)
        // and not in others. Collapse both to a single separator.
        let normalised = raw.replace(r"\\", r"\");
        // Reject a span that swallowed a quote or a field boundary —
        // that means we walked left past the start of the path.
        if normalised.contains('"') || normalised.contains(',') {
            continue;
        }
        out.push(PathBuf::from(normalised));
    }
    out
}

/// Scan the RSI Launcher's logs for install roots. Bounded work: the
/// logs are multi-megabyte, so we read each one once, line by line,
/// and stop after [`LAUNCHER_SCAN_MAX_BYTES`].
#[cfg(target_os = "windows")]
fn launcher_log_install_roots() -> Vec<PathBuf> {
    use std::io::{BufRead, BufReader, Read};

    /// Cap on how much launcher log we read looking for install roots.
    /// The file reaches ~5 MB before rotation; the path appears many
    /// times throughout, so a partial read is not a partial answer.
    const LAUNCHER_SCAN_MAX_BYTES: u64 = 4 * 1024 * 1024;

    let mut out = Vec::new();
    for root in launcher_log_roots() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }
            let Ok(file) = std::fs::File::open(&path) else {
                continue;
            };
            let mut reader = BufReader::new(file.take(LAUNCHER_SCAN_MAX_BYTES));
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                out.extend(install_roots_in_log_line(&line));
            }
        }
    }
    dedupe(out)
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

/// Resolve a user-supplied path into discovered logs.
///
/// The Settings field is labelled "Game.log override path", but the
/// path a user pastes is just as likely to be the install folder or a
/// channel folder — that's what a file browser hands them, and it's
/// what the launcher shows. Every one of these is accepted:
///
/// | What the user pasted                  | Treated as    |
/// |---------------------------------------|---------------|
/// | `…/StarCitizen/LIVE/Game.log`         | the log file  |
/// | `…/StarCitizen/LIVE`                  | a channel dir |
/// | `…/StarCitizen`                       | an install    |
/// | `…/Roberts Space Industries`          | install parent|
///
/// Returns an empty vec when the path resolves to nothing usable —
/// the caller reports that as an invalid override rather than
/// silently falling back, so the user finds out their path is wrong.
pub fn resolve_override(user_path: &Path) -> Vec<DiscoveredLog> {
    let mut out = Vec::new();

    // A file: take it as the live log, whatever it is called. The
    // channel is the parent directory's name when that names a known
    // channel, so a `…/LIVE/Game.log` override reports as LIVE rather
    // than as an unlabelled custom source.
    if let Some(size) = meta_or_skip(user_path) {
        let channel = user_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(str::to_ascii_uppercase)
            .filter(|n| CHANNELS.contains(&n.as_str()))
            .unwrap_or_else(|| CUSTOM_CHANNEL.to_string());
        out.push(DiscoveredLog {
            channel: channel.clone(),
            kind: LogKind::ChannelLive,
            path: user_path.to_path_buf(),
            size_bytes: size,
        });

        // Pick up the rotated logs and crash dumps sitting beside it.
        // Without this an override-by-file gives the live tail a
        // target but leaves the Logs pane, the rotated-log backfill
        // and the crash scanner with nothing on a custom install —
        // they all read the same discovery output.
        //
        // Any ChannelLive the walk finds is dropped: the user named
        // the live log, and a second one would compete with their pin.
        if let Some(parent) = user_path.parent() {
            let mut siblings = Vec::new();
            collect_channel(&channel, parent, &mut siblings);
            siblings.retain(|d| d.kind != LogKind::ChannelLive);
            out.append(&mut siblings);
        }
        return out;
    }

    if !user_path.is_dir() {
        return out;
    }

    // An install root: `<path>/<channel>/Game.log`.
    collect_from_root(user_path, &mut out);
    if !out.is_empty() {
        return out;
    }

    // A channel directory: `<path>/Game.log`. Label it with the
    // directory's own name when that is a known channel.
    let channel = user_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_ascii_uppercase)
        .filter(|n| CHANNELS.contains(&n.as_str()))
        .unwrap_or_else(|| CUSTOM_CHANNEL.to_string());
    collect_channel(&channel, user_path, &mut out);
    if !out.is_empty() {
        return out;
    }

    // One level up from an install root — the user pasted the
    // `Roberts Space Industries` folder, or a library dir holding
    // `StarCitizen/`. Only one level; we are resolving a path the
    // user gave us, not crawling their disk.
    if let Ok(entries) = std::fs::read_dir(user_path) {
        for entry in entries.flatten() {
            collect_from_root(&entry.path(), &mut out);
        }
    }

    out
}

/// Walk every standard install root. Used on its own by
/// [`discover`], and as the fallback when an override is set but
/// doesn't resolve.
fn discover_standard(out: &mut Vec<DiscoveredLog>) {
    for root in install_roots() {
        if !root.exists() {
            continue;
        }
        collect_from_root(&root, out);
    }
    collect_launcher_logs(out);
}

/// Discover every log artifact, honouring a user-supplied override.
///
/// When the override resolves, its logs come FIRST in the returned
/// list — [`crate::main`]'s tail picker treats a leading override
/// entry as a pin, so a user who points at PTU keeps getting PTU even
/// when LIVE holds a larger log. Standard roots are still walked
/// afterwards so the Logs pane and the rotated-log backfill still see
/// everything else on disk.
///
/// When the override does NOT resolve, the standard roots are the
/// only source and [`override_resolves`] reports false so the health
/// surface can say so.
pub fn discover_with_override(user_path: Option<&Path>) -> Vec<DiscoveredLog> {
    let mut out = Vec::new();
    if let Some(user_path) = user_path {
        out.extend(resolve_override(user_path));
    }
    let mut standard = Vec::new();
    discover_standard(&mut standard);
    // Don't list a log twice when the override points inside a root
    // we would have found anyway.
    let already: std::collections::HashSet<PathBuf> = out.iter().map(|d| d.path.clone()).collect();
    standard.retain(|d| !already.contains(&d.path));
    out.append(&mut standard);
    out
}

/// Whether a user-supplied override points at something we can
/// actually read logs from. Drives the health surface's
/// "your override path is wrong" item.
pub fn override_resolves(user_path: &Path) -> bool {
    !resolve_override(user_path).is_empty()
}

/// Choose which live log to tail.
///
/// A configured override WINS OUTRIGHT when it resolves — picking the
/// largest candidate instead would quietly ignore a user who pointed
/// at PTU while a bigger LIVE log sat next to it, which reads to them
/// exactly like the override doing nothing at all.
///
/// With no override (or one that resolves to nothing), fall back to
/// the largest discovered live log across the standard install roots.
///
/// Only `LogKind::ChannelLive` entries are considered. Discovery also
/// surfaces archived rotated logs and crash reports for UI
/// visibility, but those aren't tail-able sources — picking one
/// would mean reading a stale file with no ongoing updates.
pub fn select_tail_target(
    override_path: Option<&std::path::Path>,
    discovered: Vec<DiscoveredLog>,
) -> Option<DiscoveredLog> {
    let mut live: Vec<DiscoveredLog> = discovered
        .into_iter()
        .filter(|d| d.kind == LogKind::ChannelLive)
        .collect();

    if let Some(want) = override_path {
        // `discover_with_override` puts the override's own entries
        // first, but match on the path so the pin survives any future
        // reordering of the discovery output.
        let resolved = resolve_override(want);
        if let Some(pinned) = resolved.iter().find(|d| d.kind == LogKind::ChannelLive) {
            return Some(pinned.clone());
        }
        tracing::warn!(
            path = %want.display(),
            "configured gamelog_path does not resolve to a readable Game.log — \
             falling back to auto-discovery"
        );
    }

    live.sort_by_key(|a| std::cmp::Reverse(a.size_bytes));
    live.into_iter().next()
}

/// Discover every log artifact, honouring the configured override.
///
/// Reads `gamelog_path` from `config.toml` directly rather than via
/// [`crate::config::load`] — `load` hydrates secrets from the OS
/// keychain, and `discover` is called on every status poll.
pub fn discover() -> Vec<DiscoveredLog> {
    discover_with_override(crate::config::gamelog_path_override().as_deref())
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

    /// Host-machine smoke probe. Ignored in CI (there is no Star
    /// Citizen install on a runner); run with
    /// `cargo test -p starstats-client --bins host_probe -- --ignored --nocapture`
    /// to see what discovery actually finds on this machine.
    #[test]
    #[ignore = "depends on a real Star Citizen install on the host"]
    fn host_probe_reports_what_discovery_finds() {
        let found = discover();
        println!("--- discovered {} artifacts ---", found.len());
        for d in &found {
            println!("{:?} {} {} bytes", d.kind, d.path.display(), d.size_bytes);
        }
        println!("--- launcher-log install roots ---");
        for r in launcher_log_install_roots() {
            println!("{} (exists: {})", r.display(), r.exists());
        }
        println!("--- tail target ---");
        println!("{:?}", select_tail_target(None, found));
    }

    // ---- Override resolution -------------------------------------
    //
    // These cover the reported bug: `gamelog_path` was written by the
    // Settings pane, preserved across cloud sync, and reported to the
    // health surface — but nothing ever read it to choose a log, so a
    // user on a non-default install path had no working escape hatch.

    /// Build a full install root with two channels whose live logs
    /// differ in size, so "largest wins" and "the override wins" give
    /// visibly different answers.
    fn build_install(root: &Path) -> PathBuf {
        let install = root.join("StarCitizen");
        for (channel, body) in [("LIVE", vec![b'L'; 4_000]), ("PTU", vec![b'P'; 100])] {
            let dir = install.join(channel);
            fs::create_dir_all(&dir).unwrap();
            write_file(&dir.join("Game.log"), &body);
        }
        install
    }

    #[test]
    fn resolve_override_accepts_the_game_log_file_itself() {
        let tmp = TempDir::new().unwrap();
        let install = build_install(tmp.path());
        let out = resolve_override(&install.join("PTU").join("Game.log"));
        assert_eq!(out.len(), 1, "got {out:#?}");
        assert_eq!(out[0].kind, LogKind::ChannelLive);
        // Channel is taken from the parent directory, not invented.
        assert_eq!(out[0].channel, "PTU");
        assert_eq!(out[0].size_bytes, 100);
    }

    #[test]
    fn resolve_override_by_file_also_surfaces_the_rotated_logs_beside_it() {
        // The Logs pane, the rotated-log backfill and the crash
        // scanner all read the same discovery output as the tail. An
        // override that yielded ONLY the live log would leave every
        // one of them empty on a custom install.
        let tmp = TempDir::new().unwrap();
        let channel_dir = tmp.path().join("LIVE");
        fs::create_dir_all(channel_dir.join("logbackups")).unwrap();
        write_file(&channel_dir.join("Game.log"), b"live");
        write_file(&channel_dir.join("logbackups/Game.20260104.log"), b"old");

        let out = resolve_override(&channel_dir.join("Game.log"));

        let live: Vec<_> = out
            .iter()
            .filter(|d| d.kind == LogKind::ChannelLive)
            .collect();
        assert_eq!(
            live.len(),
            1,
            "exactly one live log, the pinned one: {out:#?}"
        );
        assert!(
            out.iter().any(|d| d.kind == LogKind::ChannelArchived),
            "rotated log beside the override was not surfaced: {out:#?}"
        );
    }

    #[test]
    fn resolve_override_accepts_a_channel_directory() {
        let tmp = TempDir::new().unwrap();
        let install = build_install(tmp.path());
        let out = resolve_override(&install.join("LIVE"));
        let live: Vec<_> = out
            .iter()
            .filter(|d| d.kind == LogKind::ChannelLive)
            .collect();
        assert_eq!(live.len(), 1, "got {out:#?}");
        assert_eq!(live[0].channel, "LIVE");
        assert!(live[0].path.ends_with("Game.log"));
    }

    #[test]
    fn resolve_override_accepts_the_install_root() {
        let tmp = TempDir::new().unwrap();
        let install = build_install(tmp.path());
        let out = resolve_override(&install);
        let mut channels: Vec<&str> = out
            .iter()
            .filter(|d| d.kind == LogKind::ChannelLive)
            .map(|d| d.channel.as_str())
            .collect();
        channels.sort_unstable();
        assert_eq!(channels, vec!["LIVE", "PTU"], "got {out:#?}");
    }

    #[test]
    fn resolve_override_accepts_the_directory_above_the_install() {
        // What a user pastes when they copy the launcher's library
        // path: `…/Roberts Space Industries`, not `…/StarCitizen`.
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path().join("Roberts Space Industries");
        fs::create_dir_all(&parent).unwrap();
        build_install(&parent);
        let out = resolve_override(&parent);
        assert!(
            out.iter().any(|d| d.kind == LogKind::ChannelLive),
            "install one level down was not found: {out:#?}"
        );
    }

    #[test]
    fn resolve_override_labels_a_loose_log_as_custom() {
        // A log copied to the desktop has no channel directory above
        // it — take it anyway rather than refusing, but don't claim a
        // channel we can't see.
        let tmp = TempDir::new().unwrap();
        let loose = tmp.path().join("Game.log");
        write_file(&loose, b"copied\n");
        let out = resolve_override(&loose);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].channel, "CUSTOM");
    }

    #[test]
    fn resolve_override_yields_nothing_for_a_path_with_no_logs() {
        let tmp = TempDir::new().unwrap();
        let empty = tmp.path().join("nothing-here");
        fs::create_dir_all(&empty).unwrap();
        assert!(resolve_override(&empty).is_empty());
        assert!(!override_resolves(&empty));
        // And for a path that doesn't exist at all — the typo case.
        assert!(!override_resolves(&tmp.path().join("does-not-exist")));
    }

    #[test]
    fn override_resolves_is_true_for_a_real_install() {
        let tmp = TempDir::new().unwrap();
        let install = build_install(tmp.path());
        assert!(override_resolves(&install));
    }

    // ---- Tail selection ------------------------------------------

    #[test]
    fn select_tail_target_pins_the_override_over_a_larger_discovered_log() {
        // The regression this whole change exists for. The user runs
        // PTU; LIVE holds a much larger log beside it. Selecting by
        // size ignores the override completely, which is
        // indistinguishable from the override doing nothing.
        let tmp = TempDir::new().unwrap();
        let install = build_install(tmp.path());
        let want = install.join("PTU").join("Game.log");

        let mut discovered = Vec::new();
        collect_from_root(&install, &mut discovered);
        assert!(
            discovered.iter().any(|d| d.size_bytes == 4_000),
            "fixture must contain a LARGER non-override log"
        );

        let picked = select_tail_target(Some(&want), discovered).expect("a target");
        assert_eq!(picked.path, want, "override did not win the selection");
        assert_eq!(picked.channel, "PTU");
    }

    #[test]
    fn select_tail_target_falls_back_to_largest_when_override_is_broken() {
        let tmp = TempDir::new().unwrap();
        let install = build_install(tmp.path());
        let mut discovered = Vec::new();
        collect_from_root(&install, &mut discovered);

        let picked =
            select_tail_target(Some(&tmp.path().join("typo")), discovered).expect("fallback");
        // Largest live log — LIVE at 4000 bytes.
        assert_eq!(picked.channel, "LIVE");
        assert_eq!(picked.size_bytes, 4_000);
    }

    #[test]
    fn select_tail_target_picks_largest_with_no_override() {
        let tmp = TempDir::new().unwrap();
        let install = build_install(tmp.path());
        let mut discovered = Vec::new();
        collect_from_root(&install, &mut discovered);
        let picked = select_tail_target(None, discovered).expect("a target");
        assert_eq!(picked.channel, "LIVE");
    }

    #[test]
    fn select_tail_target_never_picks_an_archived_or_crash_log() {
        // Archived logs and crash dumps are not tail-able — they never
        // grow again. A huge rotated log must not beat a small live one.
        let tmp = TempDir::new().unwrap();
        let channel_dir = tmp.path().join("LIVE");
        fs::create_dir_all(channel_dir.join("Logs")).unwrap();
        write_file(&channel_dir.join("Game.log"), b"tiny");
        write_file(
            &channel_dir.join("Logs/Game-20260101-120000.log"),
            &vec![b'A'; 50_000],
        );
        let mut discovered = Vec::new();
        collect_channel("LIVE", &channel_dir, &mut discovered);

        let picked = select_tail_target(None, discovered).expect("a target");
        assert_eq!(picked.kind, LogKind::ChannelLive);
        assert_eq!(picked.size_bytes, 4);
    }

    #[test]
    fn select_tail_target_returns_none_when_nothing_is_discovered() {
        assert!(select_tail_target(None, Vec::new()).is_none());
    }

    #[test]
    fn discover_with_override_puts_the_override_first_and_lists_each_log_once() {
        let tmp = TempDir::new().unwrap();
        let install = build_install(tmp.path());
        let want = install.join("PTU").join("Game.log");
        let out = discover_with_override(Some(&want));

        assert_eq!(out.first().map(|d| &d.path), Some(&want));
        // The standard-root walk must not re-list a path the override
        // already contributed.
        let hits = out.iter().filter(|d| d.path == want).count();
        assert_eq!(hits, 1, "override log listed {hits} times: {out:#?}");
    }

    // ---- Launcher-log install-root extraction --------------------

    #[test]
    fn install_root_is_read_from_a_launcher_pipeline_line() {
        // Verbatim shape from %APPDATA%/rsilauncher/logs/log.log, where
        // separators arrive doubled inside the JSON-ish field.
        let line = r#"", "[main][info] ": "[Pipeline] Installing Star Citizen LIVE 4.10.0 at C:\\Games\\Roberts Space Industries\\StarCitizen (type: update)"  },"#;
        let roots = install_roots_in_log_line(line);
        assert_eq!(
            roots,
            vec![PathBuf::from(
                r"C:\Games\Roberts Space Industries\StarCitizen"
            )],
            "got {roots:#?}"
        );
    }

    #[test]
    fn install_root_from_a_compute_size_line_drops_the_channel_segment() {
        // This line names the CHANNEL directory; the install root is
        // its parent. Both line shapes must collapse to the same root
        // or deduping the two is pointless.
        let line = r"[ComputeSizePhase] Computing required space for (SC LIVE) in D:\SC\Roberts Space Industries\StarCitizen\LIVE";
        let roots = install_roots_in_log_line(line);
        assert_eq!(
            roots,
            vec![PathBuf::from(r"D:\SC\Roberts Space Industries\StarCitizen")],
            "got {roots:#?}"
        );
    }

    #[test]
    fn launcher_line_does_not_mistake_the_executable_for_an_install_root() {
        // Observed on a real launcher log: the line names the folder
        // AND `StarCitizen.exe` inside it, and a plain substring match
        // emitted `…\LIVE\StarCitizen` as a second root.
        let line =
            r"Launching C:\Games\Roberts Space Industries\StarCitizen\LIVE\Bin64\StarCitizen.exe";
        let roots = install_roots_in_log_line(line);
        assert_eq!(
            roots,
            vec![PathBuf::from(
                r"C:\Games\Roberts Space Industries\StarCitizen"
            )],
            "got {roots:#?}"
        );
    }

    #[test]
    fn launcher_line_without_an_install_path_yields_nothing() {
        assert!(install_roots_in_log_line("[main][info] nothing to see here").is_empty());
        // A bare mention with no drive letter is not a path.
        assert!(install_roots_in_log_line("launching StarCitizen now").is_empty());
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
