//! Pure character-life FSM: segments a handle's timestamp-ordered event
//! stream into "lives" -- spans of the character being alive, bounded by
//! deaths and session gaps.
//!
//! ## Why not spawn-anchored
//!
//! An earlier design opened a life on every `ResolveSpawn`. A real-log
//! audit (2026-07-17) disproved that: `resolve_spawn` fires ~2.7x more
//! often than `player_death` (900 vs 330 in one capture) because it also
//! fires on initial login, quantum-travel arrival, and region/streaming
//! loads. 74% of deaths had NO following spawn before the next death, and
//! when one did follow the median gap was ~36 minutes (a later session's
//! login, not a respawn). So spawns are session/region-load artifacts, not
//! respawn markers, and anchoring lives on them over-segments the stream.
//!
//! ## The model
//!
//! Lives are anchored on the two RELIABLE signals: **deaths** (the closer)
//! and **session boundaries** (a >`session_gap_secs` gap). A life opens at
//! the first event of a session and re-opens at each death (the character
//! respawned and keeps playing). `ResolveSpawn` is treated as an ordinary
//! event.
//!
//! ## Active-bounded duration
//!
//! A life's `duration_secs` (which feeds the "survival streak" =
//! `longest_life_secs`) is the span between its first and last ACTIVE
//! event, NOT wall-clock end-minus-start. Passive streaming events
//! ([`is_passive`] -- planet loads, attachment spam, HUD notifications,
//! etc.) keep the *session clock* alive but do not count as "time actively
//! alive", so an AFK-but-still-logging stretch can't inflate a life. A life
//! with no active event is dropped entirely (nothing observably happened).
//!
//! Mirrors the `inference` module's purity contract: given the same input
//! event slice, `derive_lives` produces the same output, no I/O.

use crate::events::GameEvent;
use crate::inference::{envelope_timestamp, parse_ts};
use crate::wire::EventEnvelope;

/// Tunables for [`derive_lives`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LifeConfig {
    /// Gap (in seconds) between two consecutive event timestamps that
    /// closes any open life as [`LifeEnd::SessionGap`] and starts a new
    /// session count. 1800s (30 minutes) matches the repo's
    /// `count_sessions_since` idle-gap so the FSM's session count agrees
    /// with the server's canonical one.
    pub session_gap_secs: i64,
}

impl Default for LifeConfig {
    fn default() -> Self {
        Self {
            session_gap_secs: 1800,
        }
    }
}

/// Engine/streaming event types that fire without player agency. They keep
/// the session clock alive but do NOT count toward a life's active-bounded
/// duration (the "survival streak"). Everything else -- deaths, incaps,
/// spawns, quantum, inventory, missions, location changes -- is "active".
///
/// Tuned against a real capture; expand cautiously (each addition shrinks
/// measured survival time).
fn is_passive(event: &GameEvent) -> bool {
    matches!(
        event,
        GameEvent::PlanetTerrainLoad(_)
            | GameEvent::AttachmentReceived(_)
            | GameEvent::SeedSolarSystem(_)
            | GameEvent::BurstSummary(_)
            | GameEvent::HudNotification(_)
    )
}

/// How a [`Life`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeEnd {
    /// Closed by an observed or inferred `PlayerDeath`.
    Death,
    /// Closed by a `GameCrash`.
    Crash,
    /// Closed because the next event's timestamp was more than
    /// `session_gap_secs` past the previous one.
    SessionGap,
    /// Still open at the end of the stream.
    StillAlive,
}

/// One life: a span of the character being alive, opened at a session start
/// or a respawn (death) and closed by the next death / crash / session gap /
/// end-of-stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Life {
    /// When the life opened (session start, or the closing death's ts on a
    /// respawn). `None` only if that opening event had no parseable ts.
    pub start_ts: Option<String>,
    /// The closing event's ts (`Death`/`Crash`); `None` for `SessionGap`
    /// (the gap, not an event, closed it) and `StillAlive`.
    pub end_ts: Option<String>,
    /// Active-play seconds: last-active minus first-active event within the
    /// life (passive streaming excluded). `None` when the life has fewer
    /// than two active timestamps or they don't parse in order (never
    /// negative).
    pub duration_secs: Option<i64>,
    pub ended_by: LifeEnd,
    /// Downs (`PlayerIncapacitated`) survived within this life.
    pub incap_count: u32,
    /// Zone from the closing `PlayerDeath`, if any.
    pub death_zone: Option<String>,
    /// The closing death was inferred (`body_class == "inferred"`), not
    /// observed.
    pub death_inferred: bool,
}

/// Mutable builder for the currently-open life.
struct OpenLife {
    start_ts: Option<String>,
    first_active: Option<i64>,
    last_active: Option<i64>,
    incap_count: u32,
    death_zone: Option<String>,
    death_inferred: bool,
}

impl OpenLife {
    fn new(start_ts: Option<String>) -> Self {
        Self {
            start_ts,
            first_active: None,
            last_active: None,
            incap_count: 0,
            death_zone: None,
            death_inferred: false,
        }
    }

    /// Record an active event's epoch, extending the active window.
    fn mark_active(&mut self, ts: i64) {
        if self.first_active.is_none() {
            self.first_active = Some(ts);
        }
        self.last_active = Some(ts);
    }

    /// Finalize into a [`Life`], or `None` if nothing active happened
    /// (empty respawn tail / purely-passive span) -- such spans are not a
    /// countable life. A `Death`/`Crash`-ended life is ALWAYS kept, even
    /// with no active window (e.g. a death whose timestamp doesn't parse):
    /// a death is a real life-ending event and must never be dropped, or
    /// `deaths` would undercount.
    fn close(self, ended_by: LifeEnd, end_ts: Option<String>) -> Option<Life> {
        let ends_a_real_life = matches!(ended_by, LifeEnd::Death | LifeEnd::Crash);
        if self.first_active.is_none() && !ends_a_real_life {
            return None;
        }
        let duration_secs = match (self.first_active, self.last_active) {
            (Some(a), Some(b)) if b >= a => Some(b - a),
            _ => None, // out-of-order / unparseable -> never negative
        };
        Some(Life {
            start_ts: self.start_ts,
            end_ts,
            duration_secs,
            ended_by,
            incap_count: self.incap_count,
            death_zone: self.death_zone,
            death_inferred: self.death_inferred,
        })
    }
}

/// Aggregate summary returned by [`derive_lives`].
#[derive(Debug, Clone, PartialEq)]
pub struct LifeSummary {
    pub lives: Vec<Life>,
    pub total_lives: u32,
    pub deaths: u32,
    /// How many of `deaths` were INFERRED rather than observed.
    ///
    /// Always `<= deaths`. Computed in the same pass so the two cannot
    /// drift: a split derived separately would eventually disagree with
    /// the total it is meant to describe.
    ///
    /// This matters because CIG removed the Actor Death log lines, so a
    /// death is frequently reconstructed from a `Corpse` line rather
    /// than read directly. A user seeing "12 deaths" has no way to know
    /// some were reconstructed unless the split travels with the total.
    pub deaths_inferred: u32,
    pub mean_life_secs: Option<i64>,
    /// The survival streak: the longest single life's active-bounded
    /// duration.
    pub longest_life_secs: Option<i64>,
    pub sessions: u32,
    pub deaths_per_session: Option<f32>,
    pub lives_ended_by_crash: u32,
}

/// Segment a timestamp-ORDERED event stream into lives. Pure: given the
/// same input it returns the same output, no I/O.
///
/// Walks the stream once, tracking one open life + a session clock:
/// - A session opens on the first event, and re-opens after any gap greater
///   than `cfg.session_gap_secs`; each session start opens a fresh life.
/// - Every non-[`is_passive`] event extends the open life's active window
///   (for the active-bounded duration).
/// - `PlayerIncapacitated` bumps `incap_count`.
/// - `PlayerDeath` closes the open life as `Death` (recording `death_zone`
///   and `death_inferred`) and immediately re-opens a new life at the death
///   ts (the character respawned).
/// - `GameCrash` closes the open life as `Crash` and re-opens a new life.
/// - A gap greater than `cfg.session_gap_secs` closes the open life as
///   `SessionGap` and bumps the session counter.
/// - End of stream closes any open life as `StillAlive`.
/// - `ResolveSpawn` gets NO special handling -- it is an ordinary (active)
///   event (see the module docs for why).
///
/// A closed life with no active event (an empty respawn tail, or a span of
/// only passive streaming) is dropped, so deaths -- always active -- are
/// never lost while empty spans can't inflate the count.
pub fn derive_lives(events: &[EventEnvelope], cfg: &LifeConfig) -> LifeSummary {
    let mut lives: Vec<Life> = Vec::new();
    let mut open: Option<OpenLife> = None;
    let mut last_ts: Option<i64> = None;
    let mut sessions: u32 = 0;
    let mut in_session = false;

    for env in events {
        let Some(ev) = env.event.as_ref() else {
            continue;
        };
        let ts_str = envelope_timestamp(env);
        let ts = ts_str.and_then(parse_ts).map(|dt| dt.timestamp());

        // Session boundary: a gap larger than the threshold closes the open
        // life and starts a new session.
        if let (Some(cur), Some(prev)) = (ts, last_ts) {
            if cur - prev > cfg.session_gap_secs {
                if let Some(life) = open.take() {
                    if let Some(l) = life.close(LifeEnd::SessionGap, None) {
                        lives.push(l);
                    }
                }
                in_session = false;
            }
        }

        // First event of a session opens the session + its first life.
        if !in_session {
            sessions += 1;
            in_session = true;
            open = Some(OpenLife::new(ts_str.map(str::to_string)));
        }
        if let Some(t) = ts {
            last_ts = Some(t);
        }

        // Active-window bookkeeping (passive streaming excluded).
        if !is_passive(ev) {
            if let (Some(life), Some(t)) = (open.as_mut(), ts) {
                life.mark_active(t);
            }
        }

        match ev {
            GameEvent::PlayerIncapacitated(_) => {
                if let Some(life) = open.as_mut() {
                    life.incap_count += 1;
                }
            }
            GameEvent::PlayerDeath(d) => {
                let mut life = open.take().unwrap_or_else(|| OpenLife::new(None));
                life.death_zone = d.zone.clone();
                life.death_inferred = d.body_class == "inferred";
                if let Some(l) = life.close(LifeEnd::Death, ts_str.map(str::to_string)) {
                    lives.push(l);
                }
                // Respawn: a new life begins at the death instant.
                open = Some(OpenLife::new(ts_str.map(str::to_string)));
            }
            GameEvent::GameCrash(_) => {
                let life = open.take().unwrap_or_else(|| OpenLife::new(None));
                if let Some(l) = life.close(LifeEnd::Crash, ts_str.map(str::to_string)) {
                    lives.push(l);
                }
                open = Some(OpenLife::new(ts_str.map(str::to_string)));
            }
            _ => {}
        }
    }
    if let Some(life) = open.take() {
        if let Some(l) = life.close(LifeEnd::StillAlive, None) {
            lives.push(l);
        }
    }

    let deaths = lives
        .iter()
        .filter(|l| l.ended_by == LifeEnd::Death)
        .count() as u32;
    // Counted here, beside `deaths`, so the split and the total are
    // derived from one pass over one list and cannot disagree.
    let deaths_inferred = lives
        .iter()
        .filter(|l| l.ended_by == LifeEnd::Death && l.death_inferred)
        .count() as u32;
    let lives_ended_by_crash = lives
        .iter()
        .filter(|l| l.ended_by == LifeEnd::Crash)
        .count() as u32;
    let durs: Vec<i64> = lives.iter().filter_map(|l| l.duration_secs).collect();
    let mean_life_secs = (!durs.is_empty()).then(|| durs.iter().sum::<i64>() / durs.len() as i64);
    let longest_life_secs = durs.iter().max().copied();
    let deaths_per_session = (sessions > 0).then(|| deaths as f32 / sessions as f32);

    LifeSummary {
        total_lives: lives.len() as u32,
        deaths,
        deaths_inferred,
        mean_life_secs,
        longest_life_secs,
        sessions,
        deaths_per_session,
        lives_ended_by_crash,
        lives,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn deaths_inferred_counts_only_reconstructed_deaths() {
        // CIG removed the Actor Death log lines, so a death is often
        // reconstructed from a Corpse line (`body_class == "inferred"`)
        // rather than read directly. A user seeing "3 deaths" cannot know
        // some were reconstructed unless the split travels with the total.
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            death("2026-01-01T00:10:00Z", "inferred"),
            active("2026-01-01T00:11:00Z"),
            death("2026-01-01T00:20:00Z", "body_01"), // observed
            active("2026-01-01T00:21:00Z"),
            death("2026-01-01T00:30:00Z", "inferred"),
            active("2026-01-01T00:31:00Z"),
        ];
        let s = derive_lives(&evs, &LifeConfig::default());

        assert_eq!(s.deaths, 3);
        assert_eq!(s.deaths_inferred, 2);
        assert!(
            s.deaths_inferred <= s.deaths,
            "a split can never exceed the total it describes"
        );
    }

    #[test]
    fn deaths_inferred_is_zero_when_every_death_was_observed() {
        // The whole point of the split is that a fully-observed total
        // shows no provenance affordance at all.
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            death("2026-01-01T00:10:00Z", "body_01"),
            active("2026-01-01T00:11:00Z"),
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.deaths, 1);
        assert_eq!(s.deaths_inferred, 0);
    }

    use super::*;
    use crate::events::{
        GameCrash, LocationInventoryRequested, PlanetTerrainLoad, PlayerDeath, PlayerIncapacitated,
        ResolveSpawn,
    };
    use crate::wire::LogSource;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn mk(event: GameEvent) -> EventEnvelope {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        EventEnvelope {
            idempotency_key: format!("evt-{id}"),
            raw_line: format!("synthetic_{id}"),
            event: Some(event),
            source: LogSource::Live,
            source_offset: 0,
            metadata: None,
            resolved_location: None,
        }
    }

    /// An ordinary "active" (player-driven) event at `ts`.
    fn active(ts: &str) -> EventEnvelope {
        mk(GameEvent::LocationInventoryRequested(
            LocationInventoryRequested {
                timestamp: ts.into(),
                player: "alice".into(),
                location: "Stanton2_Orison".into(),
            },
        ))
    }

    /// A "passive" streaming event at `ts` (keeps the session clock alive
    /// but must not extend a life's active-bounded duration).
    fn passive(ts: &str) -> EventEnvelope {
        mk(GameEvent::PlanetTerrainLoad(PlanetTerrainLoad {
            timestamp: ts.into(),
            planet: "OOC_Stanton_2b_Daymar".into(),
        }))
    }

    fn spawn(ts: &str) -> EventEnvelope {
        mk(GameEvent::ResolveSpawn(ResolveSpawn {
            timestamp: ts.into(),
            player_geid: "g1".into(),
            fallback: false,
        }))
    }

    fn death(ts: &str, body_class: &str) -> EventEnvelope {
        mk(GameEvent::PlayerDeath(PlayerDeath {
            timestamp: ts.into(),
            body_class: body_class.into(),
            body_id: "body_id_1".into(),
            zone: None,
        }))
    }

    fn death_in_zone(ts: &str, body_class: &str, zone: &str) -> EventEnvelope {
        mk(GameEvent::PlayerDeath(PlayerDeath {
            timestamp: ts.into(),
            body_class: body_class.into(),
            body_id: "body_id_1".into(),
            zone: Some(zone.into()),
        }))
    }

    fn incap(ts: &str) -> EventEnvelope {
        mk(GameEvent::PlayerIncapacitated(PlayerIncapacitated {
            timestamp: ts.into(),
            queue_id: 1,
            zone: None,
        }))
    }

    fn crash(ts: &str) -> EventEnvelope {
        mk(GameEvent::GameCrash(GameCrash {
            timestamp: ts.into(),
            channel: "LIVE".into(),
            crash_dir_name: "crash_1".into(),
            primary_log_name: None,
            total_size_bytes: 0,
        }))
    }

    #[test]
    fn deaths_and_session_bound_lives_not_spawns() {
        // active@0 -> death@1000 -> active@1100 -> death@2000 -> active@2100 (alive).
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            death("2026-01-01T00:16:40Z", "body_01"), // +1000s
            active("2026-01-01T00:18:20Z"),           // +1100s
            death("2026-01-01T00:33:20Z", "body_01"), // +2000s
            active("2026-01-01T00:35:00Z"),           // +2100s
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 3);
        assert_eq!(s.deaths, 2);
        assert_eq!(s.sessions, 1);
        // life0: active@0..death@1000 -> 1000s; life1: active@1100..death@2000 -> 900s.
        assert_eq!(s.lives[0].duration_secs, Some(1000));
        assert!(matches!(s.lives[0].ended_by, LifeEnd::Death));
        assert_eq!(s.lives[1].duration_secs, Some(900));
        assert!(matches!(s.lives[2].ended_by, LifeEnd::StillAlive));
        assert_eq!(s.longest_life_secs, Some(1000)); // survival streak
    }

    #[test]
    fn resolve_spawn_does_not_open_or_segment_lives() {
        // The regression that motivated the rewrite: spawns are not life
        // boundaries. active@0 -> spawn -> spawn -> death@300 -> one life.
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            spawn("2026-01-01T00:01:40Z"),
            spawn("2026-01-01T00:03:20Z"),
            death("2026-01-01T00:05:00Z", "body_01"), // +300s
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 1); // NOT 3 (would be, if spawns opened lives)
        assert_eq!(s.deaths, 1);
        assert_eq!(s.lives[0].duration_secs, Some(300));
    }

    #[test]
    fn passive_streaming_does_not_extend_a_life() {
        // active@0 -> active@600 -> passive@1200 (within gap), stream end.
        // Duration is bounded by the last ACTIVE event (600), not 1200.
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            active("2026-01-01T00:10:00Z"),  // +600s
            passive("2026-01-01T00:20:00Z"), // +1200s, keeps session alive only
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 1);
        assert_eq!(s.sessions, 1); // passive@1200 was within the gap, no new session
        assert_eq!(s.lives[0].duration_secs, Some(600));
        assert!(matches!(s.lives[0].ended_by, LifeEnd::StillAlive));
    }

    #[test]
    fn purely_passive_span_is_not_a_life() {
        let evs = vec![
            passive("2026-01-01T00:00:00Z"),
            passive("2026-01-01T00:05:00Z"),
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 0);
        assert_eq!(s.sessions, 1); // a session existed, but no active life in it
    }

    #[test]
    fn incapacitation_is_counted_and_does_not_end_the_life() {
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            incap("2026-01-01T00:01:00Z"),
            incap("2026-01-01T00:02:00Z"),
            death("2026-01-01T00:03:00Z", "body_01"),
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.deaths, 1);
        assert_eq!(s.lives[0].incap_count, 2);
        assert!(matches!(s.lives[0].ended_by, LifeEnd::Death));
    }

    #[test]
    fn crash_ends_a_life_as_crash_not_death() {
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            active("2026-01-01T00:03:20Z"), // +200s
            crash("2026-01-01T00:05:00Z"),  // +300s
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 1); // trailing empty life after crash is dropped
        assert!(matches!(s.lives[0].ended_by, LifeEnd::Crash));
        assert_eq!(s.deaths, 0);
        assert_eq!(s.lives_ended_by_crash, 1);
        assert_eq!(s.lives[0].duration_secs, Some(300));
    }

    #[test]
    fn session_gap_closes_the_open_life_and_bumps_session_count() {
        // Two active events per session, a >1800s gap between sessions.
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            active("2026-01-01T00:10:00Z"), // +600s
            active("2026-01-01T01:00:00Z"), // +3600s -> gap
            active("2026-01-01T01:10:00Z"), // +4200s
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.sessions, 2);
        assert_eq!(s.total_lives, 2);
        assert!(matches!(s.lives[0].ended_by, LifeEnd::SessionGap));
        assert_eq!(s.lives[0].end_ts, None);
        assert_eq!(s.lives[0].duration_secs, Some(600));
        assert!(matches!(s.lives[1].ended_by, LifeEnd::StillAlive));
        assert_eq!(s.lives[1].duration_secs, Some(600));
    }

    #[test]
    fn lone_death_counts_and_is_never_dropped() {
        // A death is an active event, so the life containing it is kept.
        let evs = vec![death("2026-01-01T00:00:00Z", "body_01")];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 1);
        assert_eq!(s.deaths, 1);
        assert!(matches!(s.lives[0].ended_by, LifeEnd::Death));
        assert_eq!(s.lives[0].duration_secs, Some(0)); // single active event
    }

    #[test]
    fn inferred_death_sets_the_flag() {
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            death_in_zone("2026-01-01T00:01:00Z", "inferred", "Stanton"),
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert!(s.lives[0].death_inferred);
        assert_eq!(s.lives[0].death_zone.as_deref(), Some("Stanton"));
    }

    #[test]
    fn out_of_order_timestamps_yield_none_duration_never_negative() {
        // active@1000 -> death@0 -> duration None (not -1000).
        let evs = vec![
            active("2026-01-01T00:16:40Z"),           // t=1000
            death("2026-01-01T00:00:00Z", "body_01"), // t=0, before
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 1);
        assert_eq!(s.lives[0].duration_secs, None);
        assert!(matches!(s.lives[0].ended_by, LifeEnd::Death));
    }

    #[test]
    fn aggregates_mean_and_deaths_per_session() {
        // life0: active@0..death@1000 (1000s); life1: active@1100..death@1600 (500s).
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            death("2026-01-01T00:16:40Z", "body_01"), // +1000s
            active("2026-01-01T00:18:20Z"),           // +1100s
            death("2026-01-01T00:26:40Z", "body_01"), // +1600s
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.deaths, 2);
        assert_eq!(s.sessions, 1);
        assert_eq!(s.mean_life_secs, Some(750)); // (1000 + 500) / 2
        assert_eq!(s.deaths_per_session, Some(2.0));
    }

    #[test]
    fn death_with_unparseable_timestamp_is_still_counted() {
        // A death whose timestamp doesn't parse has no active window, but a
        // death is never dropped -- `deaths` must not undercount.
        let evs = vec![death("not-a-timestamp", "body_01")];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 1);
        assert_eq!(s.deaths, 1);
        assert!(matches!(s.lives[0].ended_by, LifeEnd::Death));
        assert_eq!(s.lives[0].duration_secs, None);
    }

    #[test]
    fn resolve_spawn_is_active_and_extends_duration() {
        // A spawn as the last event extends the life's active window, proving
        // ResolveSpawn is treated as active (would break if it were passive).
        let evs = vec![
            active("2026-01-01T00:00:00Z"),
            spawn("2026-01-01T00:10:00Z"), // +600s
        ];
        let s = derive_lives(&evs, &LifeConfig::default());
        assert_eq!(s.total_lives, 1);
        assert_eq!(s.lives[0].duration_secs, Some(600));
    }
}
