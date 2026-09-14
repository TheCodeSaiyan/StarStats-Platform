//! Process-detection guard for EAC-aware scheduling.
//!
//! When Star Citizen is running, the user is inside Easy Anti-Cheat's
//! protection envelope — overlapping authenticated HTTP from the same
//! machine can trip heuristics that block the user from sessions for
//! hours. The hangar fetcher consults this guard before every fetch
//! and skips the cycle if the game is up. The cost of a missed refresh
//! is bounded (the next cycle picks it up); the cost of a false
//! negative against EAC is potentially a banned account, so we err on
//! the side of skipping.

use sysinfo::{ProcessesToUpdate, System};

/// Process names we consider "Star Citizen running". Windows ships
/// the launcher as `StarCitizen.exe`; macOS / Linux builds (Wine and
/// future native, if any) have appeared without an extension. Match
/// is case-insensitive — RSI's launcher has been observed varying
/// casing across builds.
const SC_PROCESS_NAMES: &[&str] = &["StarCitizen.exe", "StarCitizen"];

/// `true` if any process whose name (case-insensitively) matches one
/// of [`SC_PROCESS_NAMES`] is currently running.
///
/// Snapshot-only — this builds a fresh `System`, refreshes the
/// process list, and discards it. The caller is responsible for
/// re-querying before each scheduled action; there is no caching
/// because the answer can flip the moment the user launches the
/// game.
///
/// `process.name()` is `&OsStr` since `sysinfo` 0.31; the comparison
/// below goes through `OsStr::eq_ignore_ascii_case`, which takes any
/// `AsRef<OsStr>`, so the `&str` targets compare without conversion.
/// `refresh_processes` grew its two arguments in 0.31 and 0.32: refresh
/// every process, and drop the ones that have exited — this is a liveness
/// probe, so a stale entry for a closed game would be exactly the wrong
/// answer.
pub fn is_starcitizen_running() -> bool {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| {
        let name = p.name();
        SC_PROCESS_NAMES
            .iter()
            .any(|target| name.eq_ignore_ascii_case(target))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // We can't assert the value — it depends entirely on whether the
    // host running the test happens to have Star Citizen open. The
    // test exists to catch panics / linker breaks / API-shape changes
    // when the `sysinfo` dependency is upgraded.
    #[test]
    fn is_starcitizen_running_returns_without_panic() {
        let _ = is_starcitizen_running();
    }
}
