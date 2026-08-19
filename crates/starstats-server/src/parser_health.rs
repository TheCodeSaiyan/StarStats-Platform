//! Parser-health detection — the pure decision layer.
//!
//! Detects that an event type has stopped being produced (or is being
//! produced at a materially reduced rate) while users remain active.
//!
//! Motivation: a `Game.log` shell tag renamed in the ~2026-07-15 patch and
//! `vehicle_stowed` fell from ~200 events/day to exactly zero for three
//! weeks. Nothing went red — no test failed, no error logged, no metric
//! moved. The break surfaced only because a user disbelieved an assistant's
//! "you have no activity in that window" explanation. This module exists so
//! the next one is caught by machinery instead of by luck.
//!
//! Everything here is pure: no clock, no database, no I/O. The window query
//! lives in [`crate::parser_health_job`] and persistence in
//! [`crate::parser_health_store`], so the decision rule can be tested
//! exhaustively against synthetic fixtures.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

/// One `(event_type, claimed_handle)` cell of the window query: how many
/// events that handle produced of that type in the recent window and in the
/// baseline window before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandleTypeCount {
    pub event_type: String,
    pub claimed_handle: String,
    pub recent_n: i64,
    pub baseline_n: i64,
    /// Latest event of this type from this handle inside the window. Rolled
    /// up per type into [`Finding::last_event_at`] — the moment the type
    /// actually stopped, which is what a candidate replacement tag's first
    /// sighting is correlated against. `first_flagged_at` cannot serve: it
    /// records when the detector NOTICED, up to a week later.
    pub last_event_at: Option<DateTime<Utc>>,
}

/// How badly a type has degraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Not a single event in the recent window. The classifier is dead.
    Dark,
    /// Still producing, but at a small fraction of its former share —
    /// e.g. a tag rename that only affects some variants of a line.
    Degraded,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Dark => "dark",
            Severity::Degraded => "degraded",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dark" => Some(Severity::Dark),
            "degraded" => Some(Severity::Degraded),
            _ => None,
        }
    }
}

/// A detected collapse, carrying the evidence that produced it so the admin
/// surface can show how much weight to give it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Finding {
    pub event_type: String,
    pub severity: Severity,
    pub baseline_events: i64,
    pub recent_events: i64,
    pub share_baseline: f64,
    pub share_recent: f64,
    /// Handles that produced this type during the baseline.
    pub baseline_handles: i64,
    /// Of those, the ones still active (any event, any type) in the recent
    /// window — users who demonstrably kept playing.
    pub carried_handles: i64,
    /// Of the carried handles, those that produced zero of this type.
    pub affected_handles: i64,
    /// When this type last fired anywhere in the window — the collapse
    /// moment. `None` only if every contributing row lacked a timestamp.
    pub last_event_at: Option<DateTime<Utc>>,
}

impl Finding {
    /// Fraction of still-active users who lost this event type. This is the
    /// simultaneity term: a parser break lands on everyone at once, a
    /// behaviour shift does not.
    pub fn affected_fraction(&self) -> f64 {
        if self.carried_handles == 0 {
            return 0.0;
        }
        self.affected_handles as f64 / self.carried_handles as f64
    }
}

/// Tunables for [`detect`]. Defaults are the shipped values; every field is
/// overridable via env so a threshold can be adjusted without a deploy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorConfig {
    /// Length of the recent window, in days.
    pub recent_days: i64,
    /// Length of the baseline window immediately preceding it, in days.
    pub baseline_days: i64,
    /// Types with fewer baseline events than this are too rare to reason
    /// about and are never flagged.
    pub min_baseline_events: i64,
    /// Recent share must fall to at most this multiple of baseline share.
    pub collapse_fraction: f64,
    /// Minimum share of carried handles that must have gone silent.
    pub min_affected_fraction: f64,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            recent_days: 7,
            baseline_days: 28,
            min_baseline_events: 200,
            collapse_fraction: 0.2,
            min_affected_fraction: 0.75,
        }
    }
}

impl DetectorConfig {
    /// Read overrides from the environment, falling back to [`Default`] for
    /// anything absent or unparseable. A malformed value is a warn, not a
    /// startup failure — a typo in one threshold must not take the API down.
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            recent_days: env_or("STARSTATS_PARSER_HEALTH_RECENT_DAYS", d.recent_days),
            baseline_days: env_or("STARSTATS_PARSER_HEALTH_BASELINE_DAYS", d.baseline_days),
            min_baseline_events: env_or(
                "STARSTATS_PARSER_HEALTH_MIN_BASELINE_EVENTS",
                d.min_baseline_events,
            ),
            collapse_fraction: env_or(
                "STARSTATS_PARSER_HEALTH_COLLAPSE_FRACTION",
                d.collapse_fraction,
            ),
            min_affected_fraction: env_or(
                "STARSTATS_PARSER_HEALTH_MIN_AFFECTED_FRACTION",
                d.min_affected_fraction,
            ),
        }
    }
}

fn env_or<T: std::str::FromStr>(key: &str, fallback: T) -> T {
    match std::env::var(key) {
        Err(_) => fallback,
        Ok(raw) => match raw.trim().parse() {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(key, value = %raw, "unparseable parser-health override; using default");
                fallback
            }
        },
    }
}

/// Per-type accumulator built while walking the query rows.
#[derive(Default)]
struct TypeAcc {
    baseline_events: i64,
    recent_events: i64,
    /// Handles that produced this type in the baseline → their recent count.
    baseline_handle_recent: HashMap<String, i64>,
    last_event_at: Option<DateTime<Utc>>,
}

/// Decide which event types have collapsed.
///
/// The rule, in order:
///
/// 1. **Share, not count.** Compare the type's share of all events in the
///    recent window against its share in the baseline. If the player base
///    simply played less, every type's count falls together and shares hold
///    steady — no finding.
/// 2. **Simultaneity.** Among handles that produced the type in the baseline
///    AND are still active in the recent window, require most to have gone
///    to zero. A parser break is a step function across all users; one
///    player losing interest is not.
///
/// Returns findings sorted by `event_type` so output is deterministic.
///
/// Degrades deliberately on a single-handle deployment: with one carried
/// handle the simultaneity term is trivially 1.0 and the rule reduces to
/// plain staleness — correct for a fleet of one, and it strengthens on its
/// own as users arrive. Requiring several handles would mean never firing on
/// the deployment that actually exists, which would be this very bug class.
pub fn detect(rows: &[HandleTypeCount], cfg: &DetectorConfig) -> Vec<Finding> {
    let total_baseline: i64 = rows.iter().map(|r| r.baseline_n).sum();
    let total_recent: i64 = rows.iter().map(|r| r.recent_n).sum();

    // No baseline means no evidence of a working past; no recent activity
    // means nobody played, so every share is undefined rather than
    // collapsed. Both are explicit guards — the second is precisely the
    // false positive this design exists to avoid.
    if total_baseline == 0 || total_recent == 0 {
        return Vec::new();
    }

    let mut active_handles: HashSet<&str> = HashSet::new();
    for r in rows {
        if r.recent_n > 0 {
            active_handles.insert(r.claimed_handle.as_str());
        }
    }

    let mut by_type: HashMap<&str, TypeAcc> = HashMap::new();
    for r in rows {
        let acc = by_type.entry(r.event_type.as_str()).or_default();
        acc.baseline_events += r.baseline_n;
        acc.recent_events += r.recent_n;
        acc.last_event_at = match (acc.last_event_at, r.last_event_at) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if r.baseline_n > 0 {
            *acc.baseline_handle_recent
                .entry(r.claimed_handle.clone())
                .or_insert(0) += r.recent_n;
        }
    }

    let mut findings: Vec<Finding> = Vec::new();
    for (event_type, acc) in by_type {
        if acc.baseline_events < cfg.min_baseline_events {
            continue;
        }

        let carried: Vec<i64> = acc
            .baseline_handle_recent
            .iter()
            .filter(|(h, _)| active_handles.contains(h.as_str()))
            .map(|(_, recent)| *recent)
            .collect();
        if carried.is_empty() {
            // Nobody who used to produce this type is still active. That is
            // churn, not a parser break — we cannot tell the difference and
            // must not guess.
            continue;
        }

        let affected = carried.iter().filter(|recent| **recent == 0).count() as i64;
        let carried_handles = carried.len() as i64;
        let affected_fraction = affected as f64 / carried_handles as f64;
        if affected_fraction < cfg.min_affected_fraction {
            continue;
        }

        let share_baseline = acc.baseline_events as f64 / total_baseline as f64;
        let share_recent = acc.recent_events as f64 / total_recent as f64;
        if share_recent > share_baseline * cfg.collapse_fraction {
            continue;
        }

        findings.push(Finding {
            event_type: event_type.to_string(),
            severity: if acc.recent_events == 0 {
                Severity::Dark
            } else {
                Severity::Degraded
            },
            baseline_events: acc.baseline_events,
            recent_events: acc.recent_events,
            share_baseline,
            share_recent,
            baseline_handles: acc.baseline_handle_recent.len() as i64,
            carried_handles,
            affected_handles: affected,
            last_event_at: acc.last_event_at,
        });
    }

    findings.sort_by(|a, b| a.event_type.cmp(&b.event_type));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ty: &str, handle: &str, recent: i64, baseline: i64) -> HandleTypeCount {
        HandleTypeCount {
            event_type: ty.to_string(),
            claimed_handle: handle.to_string(),
            recent_n: recent,
            baseline_n: baseline,
            last_event_at: None,
        }
    }

    /// Background traffic so the shares in each fixture have a denominator
    /// that behaves like a real fleet.
    fn steady_background(handles: &[&str]) -> Vec<HandleTypeCount> {
        handles
            .iter()
            .flat_map(|h| {
                vec![
                    row("hud_notification", h, 2_000, 8_000),
                    row("attachment_received", h, 1_500, 6_000),
                ]
            })
            .collect()
    }

    #[test]
    fn flags_dark_type_matching_the_vehicle_stowed_shape() {
        // The real break: ~200/day for the 28-day baseline, exactly zero
        // across the recent week, while the same users kept playing.
        let mut rows = steady_background(&["alice", "bob", "carol"]);
        for h in ["alice", "bob", "carol"] {
            rows.push(row("vehicle_stowed", h, 0, 1_900));
        }

        let found = detect(&rows, &DetectorConfig::default());

        assert_eq!(found.len(), 1, "expected exactly one finding: {found:?}");
        let f = &found[0];
        assert_eq!(f.event_type, "vehicle_stowed");
        assert_eq!(f.severity, Severity::Dark);
        assert_eq!(f.recent_events, 0);
        assert_eq!(f.carried_handles, 3);
        assert_eq!(f.affected_handles, 3);
        assert_eq!(f.affected_fraction(), 1.0);
    }

    #[test]
    fn ignores_a_quiet_week_where_everything_drops_together() {
        // Everyone played a third as much. Counts collapse, shares do not.
        // This is the false positive that makes a naive alarm worthless.
        let handles = ["alice", "bob", "carol"];
        let mut rows: Vec<HandleTypeCount> = handles
            .iter()
            .flat_map(|h| {
                vec![
                    row("hud_notification", h, 666, 8_000),
                    row("attachment_received", h, 500, 6_000),
                ]
            })
            .collect();
        for h in handles {
            rows.push(row("vehicle_stowed", h, 158, 1_900));
        }

        assert_eq!(detect(&rows, &DetectorConfig::default()), Vec::new());
    }

    #[test]
    fn ignores_one_heavy_user_leaving_while_others_continue() {
        // Alice stopped playing entirely; bob and carol still stow ships.
        // Alice is not "carried", so she cannot vote on simultaneity.
        let mut rows = steady_background(&["bob", "carol"]);
        rows.push(row("hud_notification", "alice", 0, 8_000));
        rows.push(row("vehicle_stowed", "alice", 0, 1_900));
        for h in ["bob", "carol"] {
            rows.push(row("vehicle_stowed", h, 470, 1_900));
        }

        assert_eq!(detect(&rows, &DetectorConfig::default()), Vec::new());
    }

    #[test]
    fn flags_partial_collapse_as_degraded() {
        // A tag rename that only affects some variants: the type still
        // fires for one user but is gone for the other three.
        let mut rows = steady_background(&["alice", "bob", "carol", "dave"]);
        for h in ["alice", "bob", "carol"] {
            rows.push(row("mission_objective", h, 0, 1_900));
        }
        rows.push(row("mission_objective", "dave", 40, 1_900));

        let found = detect(&rows, &DetectorConfig::default());

        assert_eq!(found.len(), 1, "expected one finding: {found:?}");
        assert_eq!(found[0].severity, Severity::Degraded);
        assert_eq!(found[0].recent_events, 40);
        assert_eq!(found[0].affected_handles, 3);
        assert_eq!(found[0].carried_handles, 4);
    }

    #[test]
    fn ignores_types_below_the_baseline_floor() {
        let mut rows = steady_background(&["alice", "bob"]);
        for h in ["alice", "bob"] {
            rows.push(row("shop_request_timed_out", h, 0, 20));
        }

        assert_eq!(detect(&rows, &DetectorConfig::default()), Vec::new());
    }

    #[test]
    fn ignores_a_window_where_nobody_was_active() {
        // Every share is undefined, not collapsed. Must not divide by zero
        // and must not flag the entire event catalogue.
        let rows: Vec<HandleTypeCount> = ["hud_notification", "vehicle_stowed"]
            .iter()
            .map(|t| row(t, "alice", 0, 5_000))
            .collect();

        assert_eq!(detect(&rows, &DetectorConfig::default()), Vec::new());
    }

    #[test]
    fn empty_input_yields_no_findings() {
        assert_eq!(detect(&[], &DetectorConfig::default()), Vec::new());
    }

    #[test]
    fn flags_on_a_single_handle_deployment() {
        // Today's fleet is effectively one handle. A rule that needs a
        // quorum would never fire here — which would be this exact bug.
        let rows = vec![
            row("hud_notification", "alice", 2_000, 8_000),
            row("vehicle_stowed", "alice", 0, 1_900),
        ];

        let found = detect(&rows, &DetectorConfig::default());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].event_type, "vehicle_stowed");
        assert_eq!(found[0].carried_handles, 1);
    }

    #[test]
    fn ignores_a_type_whose_users_all_churned() {
        // The only handle that ever produced this type is gone. Churn is
        // indistinguishable from a break here, so we say nothing.
        let mut rows = steady_background(&["bob"]);
        rows.push(row("hud_notification", "alice", 0, 8_000));
        rows.push(row("vehicle_stowed", "alice", 0, 1_900));

        assert_eq!(detect(&rows, &DetectorConfig::default()), Vec::new());
    }

    #[test]
    fn degraded_is_unreachable_on_a_single_handle_fleet() {
        // Pins a real consequence of the rule that is easy to mistake for a
        // bug later. With one carried handle `affected_fraction` is binary:
        // 1.0 when the type went silent, 0.0 the instant it produces even one
        // event. So `min_affected_fraction` (0.75) means a single-user
        // deployment can only ever see `dark` — a partial collapse cannot
        // flag until there are 4+ carried handles.
        //
        // Correct, not a gap to loosen: with one user a partial drop is
        // genuinely indistinguishable from them doing less of that activity,
        // and flagging it is the noisy alarm this design rejects. Positive
        // evidence that needs no population — a replacement tag appearing at
        // the collapse moment — is what makes small-fleet partial detection
        // possible. See `crate::unknown_tags`.
        //
        // Numbers are the real local shape: mission_objective fell to 0.28%
        // of all events from 16.0%, and must still stay silent for one user.
        let rows = vec![
            row("hud_notification", "alice", 2_000, 8_000),
            row("mission_objective", "alice", 13, 8_846),
        ];
        assert_eq!(
            detect(&rows, &DetectorConfig::default()),
            Vec::new(),
            "a lone user's partial drop must stay silent"
        );

        // Four carried handles, three silent → the gate clears at exactly
        // 0.75 and the same collapse becomes visible.
        let mut quorum = steady_background(&["alice", "bob", "carol", "dave"]);
        for h in ["alice", "bob", "carol"] {
            quorum.push(row("mission_objective", h, 0, 2_000));
        }
        quorum.push(row("mission_objective", "dave", 13, 2_000));

        let found = detect(&quorum, &DetectorConfig::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Degraded);
    }

    #[test]
    fn last_event_at_rolls_up_to_the_latest_across_handles() {
        // The collapse moment must be the newest sighting anywhere — that is
        // what a candidate replacement tag gets correlated against.
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let mut rows = steady_background(&["alice", "bob"]);
        let mut a = row("vehicle_stowed", "alice", 0, 1_000);
        a.last_event_at = Some(t("2026-07-12T00:00:00Z"));
        let mut b = row("vehicle_stowed", "bob", 0, 1_000);
        b.last_event_at = Some(t("2026-07-14T21:13:51Z"));
        rows.push(a);
        rows.push(b);

        let found = detect(&rows, &DetectorConfig::default());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].last_event_at, Some(t("2026-07-14T21:13:51Z")));
    }

    #[test]
    fn severity_round_trips_through_string_form() {
        for s in [Severity::Dark, Severity::Degraded] {
            assert_eq!(Severity::parse(s.as_str()), Some(s));
        }
        assert_eq!(Severity::parse("nonsense"), None);
    }

    #[test]
    fn affected_fraction_is_zero_when_no_handles_carried() {
        let f = Finding {
            event_type: "x".into(),
            severity: Severity::Dark,
            baseline_events: 0,
            recent_events: 0,
            share_baseline: 0.0,
            share_recent: 0.0,
            baseline_handles: 0,
            carried_handles: 0,
            affected_handles: 0,
            last_event_at: None,
        };
        assert_eq!(f.affected_fraction(), 0.0);
    }
}
