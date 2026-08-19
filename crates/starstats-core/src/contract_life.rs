//! Pure contract-run fold: turns a handle's event stream into per-contract
//! runs. Mirrors `character_life::derive_lives` -- same `&[EventEnvelope]`
//! input, same purity contract, same module shape.
//!
//! Star Citizen's HUD narrates a contract's lifecycle end-to-end via
//! `HudNotification` banners keyed by `mission_id`: `Contract Accepted:` ->
//! zero or more step-level banners (`New Objective:` / `Objective
//! Complete:`) -> a terminal `Contract Complete:` / `Contract Failed:` /
//! `Contract Withdrawn:`.
//!
//! `Contract Available:` is deliberately excluded from the contract verbs --
//! it is an offer, not an accepted run (379 of 912 raw banner groups in the
//! reference corpus are offers the player never accepted; counting them
//! would badly inflate run counts).
//!
//! ## `InProgress` vs `Unknown`
//!
//! A run still open when the event stream ends is [`ContractState::InProgress`]
//! if its last event was within `session_gap_secs` of the stream's final
//! event, and [`ContractState::Unknown`] otherwise -- idle for more than
//! the session window at capture time. (Not "the stream ended long ago" in
//! wall-clock terms: the fold only ever sees relative timestamps within
//! the log and has no way to compute that.) Collapsing the two would
//! report genuinely-live contracts as unresolved. [`ClosedBy`] is stored
//! alongside `state` so an inferred close (session end, crash, gap, shard
//! change) is never indistinguishable from an observed HUD banner.
//!
//! Every run also tracks the shard it was accepted on, a step-level
//! breakdown (`New Objective:` / `Objective Complete:` / `Objective
//! Withdrawn:` keyed by `objective_id`), and is abandoned by inference when
//! the session ends, the game crashes, the player changes shard, a session
//! gap opens, or the same mission is re-accepted (superseding the earlier
//! run). Measured on 280 real logs (609 runs): `hud_complete` 251,
//! `session_end` 134 (lumps `SessionEnd` and `GameCrash` together --
//! both closed with the same `ClosedBy` value at measurement time, before
//! they were split), stream-end 77, `superseded` 69, `shard_change` 55,
//! `hud_failed` 17, `hud_withdrawn` 6.

use crate::events::{GameEvent, HudNotification};
use crate::inference::{envelope_timestamp, parse_ts};
use crate::wire::EventEnvelope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Terminal state of one contract run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractState {
    /// Still open at the end of the stream, within the session window.
    InProgress,
    Completed,
    Failed,
    Withdrawn,
    /// Closed by inference (app exit, session gap, shard change) rather
    /// than by an observed terminal banner.
    Abandoned,
    /// Left open by a stream that ended long ago, with no closing
    /// evidence. Deliberately distinct from `InProgress`: collapsing them
    /// would report genuinely-live contracts as unresolved.
    Unknown,
    /// A later accept of the same mission_id replaced this run.
    Superseded,
}

/// WHY a run reached its state. Recorded alongside `state` so an inferred
/// close is never indistinguishable from an observed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosedBy {
    HudComplete,
    HudFailed,
    HudWithdrawn,
    SessionEnd,
    /// Closed by a `GameCrash`. Distinct from `SessionEnd` so an inferred
    /// close is never indistinguishable from another -- mirrors
    /// `character_life.rs`'s own split (`LifeEnd::Crash` separate from
    /// `SessionGap`).
    GameCrash,
    SessionGap,
    ShardChange,
    Superseded,
    /// Still open -- no close rule fired.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    InProgress,
    Complete,
    Withdrawn,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractStep {
    pub objective_id: Option<String>,
    pub order: u32,
    /// Readable HUD text, verbatim. Currently always `Some` -- every
    /// insert site in this module sets it from an HUD banner's text.
    /// Kept `Option` because a later milestone folds in `mission_objective`
    /// rows (whose text is often an unexpanded template like `Go to
    /// ~mission(Location)`), which will need `None` for steps known only
    /// from that source.
    pub text: Option<String>,
    pub state: StepState,
    pub started_at: Option<String>,
    /// When the step reached a terminal state (`Complete` or
    /// `Withdrawn`). `None` while `InProgress`.
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRun {
    pub mission_id: String,
    /// Contract name exactly as the banner gave it.
    pub name: String,
    pub state: ContractState,
    pub closed_by: ClosedBy,
    /// Always `true` -- a `ContractRun` only exists once its mission has
    /// been observed as `Contract Accepted:`. Kept as an explicit field
    /// (requested in the original spec) rather than dropped: it is
    /// constant by construction, so don't mistake it for a discriminator.
    pub accepted: bool,
    pub in_progress: bool,
    pub step_count: u32,
    pub steps_complete: u32,
    /// Defensive only: `step_count` always includes any step inserted by
    /// an unseen-step completion/withdrawal (see `complete_step` /
    /// `withdraw_step`), so `steps_complete` cannot exceed `step_count` by
    /// construction. The `saturating_sub` that computes this guards
    /// against a future regression, not something that currently fires.
    pub steps_remaining: u32,
    /// Set when an `Objective Complete:`/`Objective Withdrawn:` banner
    /// names a step this run never saw a `New Objective:` for (typically
    /// the run spans a log rotation and the step's start was in the
    /// previous file) -- the step is inserted already resolved, and this
    /// flags that the run's step history is incomplete.
    pub partial_history: bool,
    /// `join_pu.shard` in effect when the contract was accepted.
    pub connected_server: Option<String>,
    pub accepted_at: Option<String>,
    /// When the run reached its terminal state, whatever that state was
    /// (`Completed`, `Failed`, `Withdrawn`, `Abandoned`, or `Superseded`)
    /// -- NOT specifically completion. `None` while still open.
    pub closed_at: Option<String>,
    pub last_event_at: Option<String>,
    pub steps: Vec<ContractStep>,
}

/// Tunables for [`derive_contract_runs`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContractConfig {
    /// Gap between consecutive event timestamps that closes any open run
    /// as `Abandoned`/`SessionGap`. Matches `LifeConfig::session_gap_secs`
    /// so contract sessions agree with the character-life FSM.
    pub session_gap_secs: i64,
}

impl Default for ContractConfig {
    fn default() -> Self {
        Self {
            session_gap_secs: 1800,
        }
    }
}

/// Contract-level banner verbs. Step-level banners (`New Objective`,
/// `Objective Complete`, `Objective Withdrawn`) are handled separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContractVerb {
    Accepted,
    Complete,
    Failed,
    Withdrawn,
}

/// Parse a HUD banner's leading verb and return it alongside the contract
/// name, trimmed. `Contract Accepted:` has a double space after the colon,
/// and every contract-level banner ends with `: ` -- trim rather than split
/// on a fixed offset.
///
/// `Contract Available:` is intentionally NOT one of the prefixes below: it
/// is an offer the player has not accepted, not a run.
fn contract_verb(text: &str) -> Option<(ContractVerb, &str)> {
    const PREFIXES: [(&str, ContractVerb); 4] = [
        ("Contract Accepted:", ContractVerb::Accepted),
        ("Contract Complete:", ContractVerb::Complete),
        ("Contract Failed:", ContractVerb::Failed),
        ("Contract Withdrawn:", ContractVerb::Withdrawn),
    ];
    for (p, v) in PREFIXES {
        if let Some(rest) = text.strip_prefix(p) {
            return Some((v, rest.trim().trim_end_matches(':').trim()));
        }
    }
    None
}

/// The `(state, closed_by)` pair a terminal banner closes a run with.
/// `None` for `Accepted`, which opens rather than closes a run.
fn terminal_outcome(verb: ContractVerb) -> Option<(ContractState, ClosedBy)> {
    match verb {
        ContractVerb::Complete => Some((ContractState::Completed, ClosedBy::HudComplete)),
        ContractVerb::Failed => Some((ContractState::Failed, ClosedBy::HudFailed)),
        ContractVerb::Withdrawn => Some((ContractState::Withdrawn, ClosedBy::HudWithdrawn)),
        ContractVerb::Accepted => None,
    }
}

/// Step-level banner verbs, scoped by `objective_id` rather than
/// `mission_id`. Distinct namespace from [`ContractVerb`] -- `New
/// Objective:` / `Objective Complete:` / `Objective Withdrawn:` never
/// collide with the `Contract *:` prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepVerb {
    New,
    Complete,
    Withdrawn,
}

/// Parse a step banner's leading verb and return it alongside the step
/// text, trimmed the same way as [`contract_verb`].
fn step_verb(text: &str) -> Option<(StepVerb, &str)> {
    const PREFIXES: [(&str, StepVerb); 3] = [
        ("New Objective:", StepVerb::New),
        ("Objective Complete:", StepVerb::Complete),
        ("Objective Withdrawn:", StepVerb::Withdrawn),
    ];
    for (p, v) in PREFIXES {
        if let Some(rest) = text.strip_prefix(p) {
            return Some((v, rest.trim().trim_end_matches(':').trim()));
        }
    }
    None
}

/// The parser already drops this sentinel to `None` on `mission_id` /
/// `objective_id`; guarded here too so a caller that hands us raw wire
/// events (bypassing the parser) can't collapse every unrelated banner
/// into one bogus "contract".
const ZERO_GUID: &str = "00000000-0000-0000-0000-000000000000";

/// Finalize a run's derived fields and record how it closed. `closed_at`
/// is `Some` for every genuine closing event (a terminal banner, a
/// session/shard/supersede inference) and `None` for the end-of-stream
/// inference, which never actually touched the run -- `run.closed_at` and
/// `run.last_event_at` are left as whatever the last real event set them
/// to.
fn close_run(
    run: &mut ContractRun,
    state: ContractState,
    closed_by: ClosedBy,
    closed_at: Option<String>,
) {
    run.state = state;
    run.closed_by = closed_by;
    run.in_progress = state == ContractState::InProgress;
    if let Some(ts) = closed_at {
        run.closed_at = Some(ts.clone());
        run.last_event_at = Some(ts);
    }
    run.step_count = run.steps.len() as u32;
    run.steps_complete = run
        .steps
        .iter()
        .filter(|s| s.state == StepState::Complete)
        .count() as u32;
    run.steps_remaining = run.step_count.saturating_sub(run.steps_complete);
}

/// Close every open run with the same `(state, closed_by, closed_at)`,
/// draining `open` into `runs`. Used by the three "abandon everything"
/// rules: session gap, shard change, session end/crash.
fn close_all(
    open: &mut HashMap<String, ContractRun>,
    runs: &mut Vec<ContractRun>,
    state: ContractState,
    closed_by: ClosedBy,
    closed_at: Option<String>,
) {
    for (_, mut run) in open.drain() {
        close_run(&mut run, state, closed_by, closed_at.clone());
        runs.push(run);
    }
}

/// Upsert a step keyed by `objective_id` on `New Objective:`. A repeat of
/// the same id refreshes the text rather than appending a second step --
/// the game re-announces an objective's text when its detail changes
/// (e.g. a waypoint updates), and each repeat is the same step.
fn upsert_step(run: &mut ContractRun, objective_id: &str, text: &str, started_at: Option<String>) {
    if let Some(step) = run
        .steps
        .iter_mut()
        .find(|s| s.objective_id.as_deref() == Some(objective_id))
    {
        step.text = Some(text.to_string());
        return;
    }
    let order = run.steps.len() as u32;
    run.steps.push(ContractStep {
        objective_id: Some(objective_id.to_string()),
        order,
        text: Some(text.to_string()),
        state: StepState::InProgress,
        started_at,
        completed_at: None,
    });
}

/// Mark a step `Complete` on `Objective Complete:`. When no matching step
/// exists (real data: a contract spanning a log rotation completes an
/// objective whose start was in the previous file), insert one already
/// `Complete` and flag `partial_history` so the gap is never silently
/// hidden.
fn complete_step(
    run: &mut ContractRun,
    objective_id: &str,
    text: &str,
    completed_at: Option<String>,
) {
    if let Some(step) = run
        .steps
        .iter_mut()
        .find(|s| s.objective_id.as_deref() == Some(objective_id))
    {
        step.state = StepState::Complete;
        step.completed_at = completed_at;
        return;
    }
    let order = run.steps.len() as u32;
    run.steps.push(ContractStep {
        objective_id: Some(objective_id.to_string()),
        order,
        text: Some(text.to_string()),
        state: StepState::Complete,
        started_at: None,
        completed_at,
    });
    run.partial_history = true;
}

/// Mark a step `Withdrawn` on `Objective Withdrawn:`, recording when it
/// resolved -- symmetric with `complete_step`, which records its
/// resolution timestamp in the same `completed_at` field (there is no
/// separate "withdrawn_at": a step has exactly one resolution timestamp,
/// whichever terminal state it resolved to). Same unseen-step handling
/// as `complete_step`: when no matching step exists (a contract spanning
/// a log rotation, withdrawing an objective whose start was in the
/// previous file -- 13 of 60 mission-linked `Objective Withdrawn`
/// banners in the reference corpus land on an unseen step), insert one
/// already `Withdrawn` and flag `partial_history` so the gap is never
/// silently dropped.
fn withdraw_step(
    run: &mut ContractRun,
    objective_id: &str,
    text: &str,
    resolved_at: Option<String>,
) {
    if let Some(step) = run
        .steps
        .iter_mut()
        .find(|s| s.objective_id.as_deref() == Some(objective_id))
    {
        step.state = StepState::Withdrawn;
        step.completed_at = resolved_at;
        return;
    }
    let order = run.steps.len() as u32;
    run.steps.push(ContractStep {
        objective_id: Some(objective_id.to_string()),
        order,
        text: Some(text.to_string()),
        state: StepState::Withdrawn,
        started_at: None,
        completed_at: resolved_at,
    });
    run.partial_history = true;
}

/// Handle a single `HudNotification`, mutating `open`/`runs` in place.
/// Zero-GUID and missionless notifications are not contracts and are
/// ignored outright.
fn handle_hud(
    hud: &HudNotification,
    ts: Option<&str>,
    current_shard: &Option<String>,
    open: &mut HashMap<String, ContractRun>,
    runs: &mut Vec<ContractRun>,
) {
    let Some(mission_id) = hud.mission_id.as_deref() else {
        return;
    };
    if mission_id == ZERO_GUID {
        return;
    }

    if let Some((verb, name)) = contract_verb(&hud.text) {
        if verb == ContractVerb::Accepted {
            if let Some(mut existing) = open.remove(mission_id) {
                close_run(
                    &mut existing,
                    ContractState::Superseded,
                    ClosedBy::Superseded,
                    ts.map(str::to_string),
                );
                runs.push(existing);
            }
            open.insert(
                mission_id.to_string(),
                ContractRun {
                    mission_id: mission_id.to_string(),
                    name: name.to_string(),
                    state: ContractState::InProgress,
                    closed_by: ClosedBy::None,
                    accepted: true,
                    in_progress: true,
                    step_count: 0,
                    steps_complete: 0,
                    steps_remaining: 0,
                    partial_history: false,
                    connected_server: current_shard.clone(),
                    accepted_at: ts.map(str::to_string),
                    closed_at: None,
                    last_event_at: ts.map(str::to_string),
                    steps: Vec::new(),
                },
            );
            return;
        }

        if let Some((state, closed_by)) = terminal_outcome(verb) {
            if let Some(mut run) = open.remove(mission_id) {
                close_run(&mut run, state, closed_by, ts.map(str::to_string));
                runs.push(run);
            }
            // No `else`: a terminal banner with no open run for its
            // mission_id (e.g. the accept was in a previous log file) has
            // nothing to close and is silently dropped here. Unlike
            // `complete_step`/`withdraw_step`, which handle the same
            // log-rotation shape one level down by synthesizing a step
            // and flagging `partial_history`, this is NOT synthesized --
            // measured across 280 real logs, 274 terminal banners had a
            // preceding accept and 0 did not, so the drop is unreachable
            // on real data. Documented rather than fixed: do not add
            // synthesis logic for a case that doesn't occur.
        }
        return;
    }

    // Step banners only mean anything against an already-open run -- an
    // objective update for a contract we never saw accepted (e.g. its
    // accept was in a previous log file) has nowhere to attach.
    let Some((verb, text)) = step_verb(&hud.text) else {
        return;
    };
    let Some(objective_id) = hud.objective_id.as_deref() else {
        return;
    };
    let Some(run) = open.get_mut(mission_id) else {
        return;
    };
    match verb {
        StepVerb::New => upsert_step(run, objective_id, text, ts.map(str::to_string)),
        StepVerb::Complete => complete_step(run, objective_id, text, ts.map(str::to_string)),
        StepVerb::Withdrawn => withdraw_step(run, objective_id, text, ts.map(str::to_string)),
    }
    run.last_event_at = ts.map(str::to_string);
}

/// Segment a timestamp-ORDERED event stream into per-contract runs. Pure:
/// given the same input it returns the same output, no I/O.
///
/// Walks the stream once, holding open runs keyed by `mission_id` (since
/// multiple contracts can be in progress at once) and tracking `last_ts`
/// (the previous event's timestamp) and `current_shard` (the shard of the
/// most recent `JoinPu`):
/// - **Session gap**: before handling an event, if the gap since `last_ts`
///   exceeds `cfg.session_gap_secs`, every open run is closed
///   `Abandoned`/`SessionGap` at `last_ts`.
/// - **`JoinPu`**: if the shard changes from the one in effect, every open
///   run is closed `Abandoned`/`ShardChange` first. Re-joining the *same*
///   shard is a no-op.
/// - **`SessionEnd`**: every open run is closed `Abandoned`/`SessionEnd`.
/// - **`GameCrash`**: every open run is closed `Abandoned`/`GameCrash` --
///   a crash abandons contracts just as a clean exit does, but keeps
///   distinct provenance from a clean session end.
/// - **`HudNotification`** with a non-zero `mission_id`:
///   - `Contract Accepted:` closes any already-open run for that mission
///     as `Superseded`/`Superseded`, then opens a new run capturing the
///     shard in effect and the accept timestamp.
///   - `Contract Complete:`/`Failed:`/`Withdrawn:` closes the open run
///     with the matching state.
///   - `Contract Available:` and any other unrecognized text are ignored.
///   - Step banners (`New Objective:`/`Objective Complete:`/`Objective
///     Withdrawn:`) update the open run's step list, keyed by
///     `objective_id`; they are ignored if no run is open.
/// - **End of stream**: every still-open run closes `InProgress`/`None`
///   if its last event is within `session_gap_secs` of the final event
///   timestamp, else `Unknown`/`None` -- collapsing this into a single
///   state would report genuinely-live contracts as unresolved.
///
/// Returns runs sorted by `accepted_at` then `mission_id` so the result is
/// stable regardless of the open-run `HashMap`'s iteration order.
pub fn derive_contract_runs(events: &[EventEnvelope], cfg: &ContractConfig) -> Vec<ContractRun> {
    let mut runs: Vec<ContractRun> = Vec::new();
    let mut open: HashMap<String, ContractRun> = HashMap::new();
    let mut last_ts: Option<i64> = None;
    let mut last_ts_str: Option<String> = None;
    let mut current_shard: Option<String> = None;

    for env in events {
        let Some(ev) = env.event.as_ref() else {
            continue;
        };
        let ts_str = envelope_timestamp(env);
        let ts = ts_str.and_then(parse_ts).map(|dt| dt.timestamp());

        if let (Some(cur), Some(prev)) = (ts, last_ts) {
            if cur - prev > cfg.session_gap_secs {
                close_all(
                    &mut open,
                    &mut runs,
                    ContractState::Abandoned,
                    ClosedBy::SessionGap,
                    last_ts_str.clone(),
                );
            }
        }

        match ev {
            GameEvent::JoinPu(join) => {
                if let Some(prev) = current_shard.as_deref() {
                    if prev != join.shard {
                        close_all(
                            &mut open,
                            &mut runs,
                            ContractState::Abandoned,
                            ClosedBy::ShardChange,
                            ts_str.map(str::to_string),
                        );
                    }
                }
                current_shard = Some(join.shard.clone());
            }
            // Both events end the play session the same way from a
            // contract's perspective -- whatever was open didn't get a
            // terminal banner and never will, so it's abandoned -- but
            // keep distinct `ClosedBy` provenance so an inferred close is
            // never indistinguishable from another. Mirrors
            // `character_life.rs`'s own split (`LifeEnd::Crash` separate
            // from `SessionGap`).
            GameEvent::SessionEnd(_) => {
                close_all(
                    &mut open,
                    &mut runs,
                    ContractState::Abandoned,
                    ClosedBy::SessionEnd,
                    ts_str.map(str::to_string),
                );
            }
            GameEvent::GameCrash(_) => {
                close_all(
                    &mut open,
                    &mut runs,
                    ContractState::Abandoned,
                    ClosedBy::GameCrash,
                    ts_str.map(str::to_string),
                );
            }
            GameEvent::HudNotification(hud) => {
                handle_hud(hud, ts_str, &current_shard, &mut open, &mut runs);
            }
            _ => {}
        }

        if let Some(t) = ts {
            last_ts = Some(t);
        }
        if let Some(s) = ts_str {
            last_ts_str = Some(s.to_string());
        }
    }

    for (_, mut run) in open.into_iter() {
        let stays_in_progress = match (run.last_event_at.as_deref().and_then(parse_ts), last_ts) {
            (Some(run_dt), Some(final_ts)) => final_ts - run_dt.timestamp() <= cfg.session_gap_secs,
            _ => false,
        };
        let state = if stays_in_progress {
            ContractState::InProgress
        } else {
            ContractState::Unknown
        };
        close_run(&mut run, state, ClosedBy::None, None);
        runs.push(run);
    }

    runs.sort_by(|a, b| (&a.accepted_at, &a.mission_id).cmp(&(&b.accepted_at, &b.mission_id)));
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{GameCrash, HudNotification, JoinPu, SessionEnd, SessionEndKind};
    use crate::wire::LogSource;

    const MID: &str = "7de35808-d909-4a6d-affe-edadf3e6fe77";
    const OID_A: &str = "2432e890-93a3-0c46-a8a5-c7bb4915881f";
    const OID_B: &str = "3543f9a1-93a3-0c46-a8a5-c7bb4915882a";

    // EventEnvelope does NOT derive Default -- all seven fields are
    // required. This mirrors character_life.rs's own `mk()` helper
    // (character_life.rs:325-335); keep them in sync.
    fn env(seq: i64, _ts: &str, event: GameEvent) -> EventEnvelope {
        EventEnvelope {
            idempotency_key: format!("evt-{seq}"),
            raw_line: format!("synthetic_{seq}"),
            event: Some(event),
            source: LogSource::Live,
            source_offset: 0,
            metadata: None,
            resolved_location: None,
        }
    }

    fn hud(
        seq: i64,
        ts: &str,
        text: &str,
        mission: Option<&str>,
        objective: Option<&str>,
    ) -> EventEnvelope {
        env(
            seq,
            ts,
            GameEvent::HudNotification(HudNotification {
                timestamp: ts.to_string(),
                text: text.to_string(),
                notification_id: seq as u64,
                mission_id: mission.map(|s| s.to_string()),
                objective_id: objective.map(|s| s.to_string()),
            }),
        )
    }

    fn join_pu(seq: i64, ts: &str, shard: &str) -> EventEnvelope {
        env(
            seq,
            ts,
            GameEvent::JoinPu(JoinPu {
                timestamp: ts.to_string(),
                address: "127.0.0.1".to_string(),
                port: 64310,
                shard: shard.to_string(),
                location_id: "0".to_string(),
            }),
        )
    }

    fn game_crash(seq: i64, ts: &str) -> EventEnvelope {
        env(
            seq,
            ts,
            GameEvent::GameCrash(GameCrash {
                timestamp: ts.to_string(),
                channel: "LIVE".to_string(),
                crash_dir_name: format!("crash-{seq}"),
                primary_log_name: None,
                total_size_bytes: 0,
            }),
        )
    }

    #[test]
    fn accepted_then_completed_yields_one_completed_run() {
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:57:59Z",
                "Contract Accepted:  Combat Gauntlet - Scenario #5: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T14:03:42Z",
                "Contract Complete: Combat Gauntlet - Scenario #5: ",
                Some(MID),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.name, "Combat Gauntlet - Scenario #5");
        assert_eq!(r.state, ContractState::Completed);
        assert_eq!(r.closed_by, ClosedBy::HudComplete);
        assert!(r.accepted);
        assert!(!r.in_progress);
        assert!(r.accepted_at.is_some() && r.closed_at.is_some());
    }

    #[test]
    fn accepted_then_failed_yields_one_failed_run() {
        // `Contract Failed:` is real (17 of 609 real runs) but had zero
        // test coverage before this fix -- a typo in its prefix at
        // `contract_verb`'s PREFIXES table would have shipped green.
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:57:59Z",
                "Contract Accepted:  Combat Gauntlet - Scenario #5: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T14:03:42Z",
                "Contract Failed: Combat Gauntlet - Scenario #5: ",
                Some(MID),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.name, "Combat Gauntlet - Scenario #5");
        assert_eq!(r.state, ContractState::Failed);
        assert_eq!(r.closed_by, ClosedBy::HudFailed);
        assert!(r.accepted);
        assert!(!r.in_progress);
        assert!(r.accepted_at.is_some() && r.closed_at.is_some());
    }

    #[test]
    fn accepted_then_withdrawn_yields_one_withdrawn_run() {
        // `Contract Withdrawn:` is real (6 of 609 real runs) but had zero
        // test coverage before this fix -- same risk as the Failed case.
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:57:59Z",
                "Contract Accepted:  Combat Gauntlet - Scenario #5: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T14:03:42Z",
                "Contract Withdrawn: Combat Gauntlet - Scenario #5: ",
                Some(MID),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.name, "Combat Gauntlet - Scenario #5");
        assert_eq!(r.state, ContractState::Withdrawn);
        assert_eq!(r.closed_by, ClosedBy::HudWithdrawn);
        assert!(r.accepted);
        assert!(!r.in_progress);
        assert!(r.accepted_at.is_some() && r.closed_at.is_some());
    }

    #[test]
    fn session_end_abandons_an_open_run() {
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:57:59Z",
                "Contract Accepted:  Test Contract: ",
                Some(MID),
                None,
            ),
            // SessionEndKind has exactly two variants: SystemQuit and
            // FastShutdown. There is no `Quit`.
            env(
                2,
                "2026-07-26T13:59:00Z",
                GameEvent::SessionEnd(SessionEnd {
                    timestamp: "2026-07-26T13:59:00Z".into(),
                    kind: SessionEndKind::SystemQuit,
                }),
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].state, ContractState::Abandoned);
        assert_eq!(runs[0].closed_by, ClosedBy::SessionEnd);
    }

    #[test]
    fn joining_a_different_shard_abandons_an_open_run() {
        // A1 named this rule explicitly; it fires on 55 of 609 real runs.
        let evs = vec![
            join_pu(1, "2026-07-26T13:50:00Z", "pub_euw1b_1_010"),
            hud(
                2,
                "2026-07-26T13:57:59Z",
                "Contract Accepted:  Test Contract: ",
                Some(MID),
                None,
            ),
            join_pu(3, "2026-07-26T14:10:00Z", "pub_euw1b_1_020"),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs[0].state, ContractState::Abandoned);
        assert_eq!(runs[0].closed_by, ClosedBy::ShardChange);
        // The shard in effect at ACCEPT time, not the one joined later.
        assert_eq!(runs[0].connected_server.as_deref(), Some("pub_euw1b_1_010"));
    }

    #[test]
    fn rejoining_the_same_shard_does_not_abandon() {
        let evs = vec![
            join_pu(1, "2026-07-26T13:50:00Z", "pub_euw1b_1_010"),
            hud(
                2,
                "2026-07-26T13:57:59Z",
                "Contract Accepted:  Test Contract: ",
                Some(MID),
                None,
            ),
            join_pu(3, "2026-07-26T14:10:00Z", "pub_euw1b_1_010"),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs[0].state, ContractState::InProgress);
        assert_eq!(runs[0].closed_by, ClosedBy::None);
    }

    #[test]
    fn a_gap_longer_than_the_session_window_abandons() {
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:00:00Z",
                "Contract Accepted:  Test Contract: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T14:00:00Z",
                "Contract Accepted:  Other: ",
                Some("other-mission"),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        let first = runs
            .iter()
            .find(|r| r.mission_id == MID)
            .expect("first run");
        assert_eq!(first.state, ContractState::Abandoned);
        assert_eq!(first.closed_by, ClosedBy::SessionGap);
    }

    #[test]
    fn run_open_at_stream_end_within_gap_stays_in_progress() {
        // A run's last touch is 1000s before the stream's final event --
        // within `session_gap_secs` (1800), so the end-of-stream check
        // (contract_life.rs's own end-of-stream branch, not the
        // mid-stream session-gap `close_all`) must classify it
        // `InProgress`, not `Unknown`. Pins the near side of the boundary
        // that `run_open_at_stream_end_past_gap_becomes_unknown` pins the
        // far side of.
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:00:00Z",
                "Contract Accepted:  Test: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T13:16:40Z", // +1000s
                "Contract Accepted:  Other: ",
                Some("other-mission"),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        let first = runs
            .iter()
            .find(|r| r.mission_id == MID)
            .expect("first run");
        assert_eq!(first.state, ContractState::InProgress);
        assert_eq!(first.closed_by, ClosedBy::None);
    }

    #[test]
    fn run_open_at_stream_end_past_gap_becomes_unknown() {
        // `ContractState::Unknown` (contract_life.rs:535's only
        // occurrence before this fix) had zero coverage -- replacing the
        // whole end-of-stream branch with an unconditional `InProgress`
        // left every other test green.
        //
        // Two hops of 1000s each: neither individually exceeds
        // `session_gap_secs` (1800), so the mid-stream session-gap
        // `close_all` never fires -- but MID's run is never touched
        // again after event 1, so by the time the stream ends 2000s have
        // passed since its last event, past the 1800s window. That must
        // resolve through the end-of-stream idle check, not the
        // mid-stream gap check.
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:00:00Z",
                "Contract Accepted:  Test: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T13:16:40Z", // +1000s
                "Contract Accepted:  Other: ",
                Some("other-mission"),
                None,
            ),
            hud(
                3,
                "2026-07-26T13:33:20Z", // +1000s more (+2000s from MID)
                "New Objective: Other step: ",
                Some("other-mission"),
                Some(OID_B),
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        let first = runs
            .iter()
            .find(|r| r.mission_id == MID)
            .expect("first run");
        assert_eq!(first.state, ContractState::Unknown);
        assert_eq!(first.closed_by, ClosedBy::None);
    }

    #[test]
    fn re_accepting_the_same_mission_supersedes_the_earlier_run() {
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:00:00Z",
                "Contract Accepted:  Test Contract: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T13:05:00Z",
                "Contract Accepted:  Test Contract: ",
                Some(MID),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].state, ContractState::Superseded);
        assert_eq!(runs[0].closed_by, ClosedBy::Superseded);
        assert_eq!(runs[1].state, ContractState::InProgress);
    }

    #[test]
    fn contract_available_is_not_a_run() {
        // 379 of 912 raw groups in the corpus are offers never accepted.
        let evs = vec![hud(
            1,
            "2026-07-26T13:00:00Z",
            "Contract Available: Some Offer: ",
            Some(MID),
            None,
        )];
        assert!(derive_contract_runs(&evs, &ContractConfig::default()).is_empty());
    }

    #[test]
    fn steps_are_counted_by_distinct_objective_id() {
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:00:00Z",
                "Contract Accepted:  Test: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T13:01:00Z",
                "New Objective: Go to Euterpe: ",
                Some(MID),
                Some(OID_A),
            ),
            // Same objective announced twice — ONE step, not two.
            hud(
                3,
                "2026-07-26T13:02:00Z",
                "New Objective: Go to Euterpe: ",
                Some(MID),
                Some(OID_A),
            ),
            hud(
                4,
                "2026-07-26T13:03:00Z",
                "Objective Complete: Go to Euterpe",
                Some(MID),
                Some(OID_A),
            ),
            hud(
                5,
                "2026-07-26T13:04:00Z",
                "Contract Complete: Test: ",
                Some(MID),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs[0].step_count, 1);
        assert_eq!(runs[0].steps_complete, 1);
        assert_eq!(runs[0].steps_remaining, 0);
        assert!(!runs[0].partial_history);
        assert_eq!(runs[0].steps[0].text.as_deref(), Some("Go to Euterpe"));
    }

    #[test]
    fn completing_a_step_never_started_flags_partial_history() {
        // Real data: a contract spanning a log rotation completes an
        // objective whose start was in the previous file.
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:00:00Z",
                "Contract Accepted:  Test: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T13:01:00Z",
                "Objective Complete: Unseen step",
                Some(MID),
                Some(OID_A),
            ),
            hud(
                3,
                "2026-07-26T13:02:00Z",
                "Contract Complete: Test: ",
                Some(MID),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert!(runs[0].partial_history);
        // Saturating — never negative, never wraps.
        assert_eq!(runs[0].steps_remaining, 0);
    }

    #[test]
    fn withdrawing_a_step_never_started_flags_partial_history() {
        // Real data: 13 of 60 mission-linked `Objective Withdrawn` banners
        // land on a step whose start was in a previous log file -- same
        // log-rotation shape as the `Objective Complete` case above.
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:00:00Z",
                "Contract Accepted:  Test: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T13:01:00Z",
                "Objective Withdrawn: Unseen step",
                Some(MID),
                Some(OID_A),
            ),
            hud(
                3,
                "2026-07-26T13:02:00Z",
                "Contract Complete: Test: ",
                Some(MID),
                None,
            ),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert!(runs[0].partial_history);
        assert_eq!(runs[0].step_count, 1);
        // Withdrawn, not completed -- resolved but not counted as done.
        assert_eq!(runs[0].steps_complete, 0);
        assert_eq!(runs[0].steps[0].state, StepState::Withdrawn);
        // Resolution timestamp now flows through symmetrically with
        // `complete_step` -- previously `withdraw_step` took no
        // timestamp and this was always `None`.
        assert_eq!(
            runs[0].steps[0].completed_at.as_deref(),
            Some("2026-07-26T13:01:00Z")
        );
    }

    #[test]
    fn game_crash_abandons_an_open_run() {
        // GameCrash shares SessionEnd's abandon-everything rule -- a
        // crash abandons open contracts just as a clean exit does -- but
        // records distinct `ClosedBy` provenance rather than collapsing
        // into `ClosedBy::SessionEnd`.
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:57:59Z",
                "Contract Accepted:  Test Contract: ",
                Some(MID),
                None,
            ),
            game_crash(2, "2026-07-26T13:59:00Z"),
        ];
        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].state, ContractState::Abandoned);
        assert_eq!(runs[0].closed_by, ClosedBy::GameCrash);
    }

    #[test]
    fn concurrent_missions_stay_isolated_and_sorted() {
        // Every other test in this module has at most one run open at a
        // time, so `runs.sort_by` (:595) and the multi-entry drains
        // (`close_all`'s `open.drain()` at :282, the end-of-stream loop
        // at :581) were untested.
        //
        // An earlier version of this test used only TWO concurrent
        // missions and asserted a specific output order. That version
        // passed even with `runs.sort_by` deleted: `HashMap`'s
        // per-instantiation randomized iteration order coincidentally
        // produced the same order the sort would have -- a 50/50 shot
        // with two entries, and this file's `derive_contract_runs`
        // builds a fresh `HashMap` per call, so the "luck" isn't even
        // stable across runs. Eight concurrent missions make a
        // coincidental full-sort match a 1-in-40320 event instead,
        // low enough to reliably catch the regression -- confirmed by
        // deleting `runs.sort_by` and re-running this test (see the
        // fix-final-report for the mutation-test transcript).
        let mission_ids: Vec<String> = (0..8).map(|i| format!("mission-{i}")).collect();
        let mut evs = Vec::new();
        let mut seq = 1i64;
        for (i, mid) in mission_ids.iter().enumerate() {
            let ts = format!("2026-07-26T13:{i:02}:00Z");
            evs.push(hud(
                seq,
                &ts,
                &format!("Contract Accepted:  Mission {i}: "),
                Some(mid.as_str()),
                None,
            ));
            seq += 1;
            evs.push(hud(
                seq,
                &ts,
                &format!("New Objective: Step {i}: "),
                Some(mid.as_str()),
                Some(format!("obj-{i}").as_str()),
            ));
            seq += 1;
        }
        evs.push(env(
            seq,
            "2026-07-26T13:20:00Z",
            GameEvent::SessionEnd(SessionEnd {
                timestamp: "2026-07-26T13:20:00Z".into(),
                kind: SessionEndKind::SystemQuit,
            }),
        ));

        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs.len(), 8);

        // Sorted by (accepted_at, mission_id) -- missions were accepted
        // in ascending order, so the output must match that exactly,
        // regardless of the open-run HashMap's iteration order.
        let got: Vec<&str> = runs.iter().map(|r| r.mission_id.as_str()).collect();
        let expected: Vec<&str> = mission_ids.iter().map(String::as_str).collect();
        assert_eq!(got, expected);

        // Steps never cross-attached between the concurrently-open runs.
        for (i, r) in runs.iter().enumerate() {
            assert_eq!(r.state, ContractState::Abandoned);
            assert_eq!(r.closed_by, ClosedBy::SessionEnd);
            assert_eq!(r.step_count, 1);
            assert_eq!(
                r.steps[0].text.as_deref(),
                Some(format!("Step {i}").as_str())
            );
        }
    }

    /// Golden test: drives one real captured contract run through the
    /// **actual** parser (`structural_parse` + `classify`), not
    /// hand-built `GameEvent`s, to catch any mismatch between what the
    /// parser emits and what this fold expects. Tasks 1-2's coverage
    /// was entirely synthetic; on this branch's predecessor that let
    /// two Critical defects ship past a fully green suite.
    ///
    /// Fixture: `tests/fixtures/contract_combat_gauntlet.txt`, four
    /// real HUD banners for one mission (accept -> objective complete
    /// with no preceding "New Objective" -> a different objective
    /// opened and never resolved -> contract complete). This naturally
    /// exercises the complete-unseen path (`partial_history`) with
    /// real data rather than a synthetic case.
    #[test]
    fn real_capture_combat_gauntlet_run_matches_derivation() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("tests/fixtures/contract_combat_gauntlet.txt");
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {path:?} must be readable: {e}"));

        let evs: Vec<EventEnvelope> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
            .map(|(i, line)| {
                let parsed = crate::parser::structural_parse(line)
                    .unwrap_or_else(|| panic!("fixture line {i} must structurally parse: {line}"));
                let event = crate::parser::classify(&parsed)
                    .unwrap_or_else(|| panic!("fixture line {i} must classify: {line}"));
                env(i as i64, parsed.timestamp, event)
            })
            .collect();
        // Fail loudly rather than pass vacuously if the fixture goes
        // missing or empty, or a line silently fails to classify.
        assert_eq!(
            evs.len(),
            4,
            "expected 4 real captured banner lines to parse and classify"
        );

        let runs = derive_contract_runs(&evs, &ContractConfig::default());
        assert_eq!(runs.len(), 1, "expected exactly one run: {runs:#?}");
        let r = &runs[0];

        assert_eq!(r.name, "Combat Gauntlet - Scenario #4");
        assert_eq!(r.state, ContractState::Completed);
        assert_eq!(r.closed_by, ClosedBy::HudComplete);
        assert_eq!(r.step_count, 2);
        assert_eq!(r.steps_complete, 1);
        assert_eq!(r.steps_remaining, 1);
        assert!(
            r.partial_history,
            "Objective Complete for 813bd6d9 arrives with no preceding New Objective for that id"
        );
        assert!(
            r.steps_remaining <= r.step_count,
            "steps_remaining must never exceed step_count"
        );
        assert_eq!(r.accepted_at.as_deref(), Some("2026-06-01T18:20:30.238Z"));
        assert_eq!(r.closed_at.as_deref(), Some("2026-06-01T18:23:33.251Z"));
    }

    #[test]
    fn output_is_deterministic_for_the_same_input() {
        let evs = vec![
            hud(
                1,
                "2026-07-26T13:00:00Z",
                "Contract Accepted:  Test: ",
                Some(MID),
                None,
            ),
            hud(
                2,
                "2026-07-26T13:01:00Z",
                "New Objective: Step: ",
                Some(MID),
                Some(OID_A),
            ),
            hud(
                3,
                "2026-07-26T13:02:00Z",
                "Contract Complete: Test: ",
                Some(MID),
                None,
            ),
        ];
        let cfg = ContractConfig::default();
        assert_eq!(
            derive_contract_runs(&evs, &cfg),
            derive_contract_runs(&evs, &cfg),
            "the fold must be pure: same slice in, identical output"
        );
    }
}
