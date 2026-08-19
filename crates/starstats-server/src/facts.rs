//! Player Facts — fun, defensible observations derived from a player's own
//! telemetry (issue #368, Tier 1).
//!
//! Distinct from the `records` widget on purpose. `records` ships
//! **superlatives** (longest session, biggest trade). Facts ship
//! **observations** — patterns, ratios, distributions. If a proposed fact is
//! a superlative it belongs in `records`; without that line the two features
//! converge and duplicate.
//!
//! Everything here is pure. [`derive_facts`] takes one assembled
//! [`FactInput`] and returns claims — no clock, no database. All the risk
//! lives in the rules and none of it needs a store to test.
//!
//! # Two invariants that are structural, not conventions
//!
//! **A Fact cannot exist without a baseline.** [`FactEvidence`] has no
//! constructor that omits one, so "every number gets a comparison" is
//! enforced by the type rather than by remembering. A bare figure is not a
//! fact, it is trivia.
//!
//! **The catalogue is fixed and pre-declared — never a search.** Every rule
//! below was chosen before the data was seen. Search a player's telemetry for
//! whatever correlates and you will always find something: at 40 candidate
//! relationships and p<0.05 you expect ~2 false positives from noise alone. A
//! fixed catalogue cannot fish.
//!
//! # Clock-time facts require a stored timezone
//!
//! "78% of your play is after 22:00" is only true in the player's own zone.
//! Derived in UTC it is quietly wrong for most of the planet, and a
//! confidently-wrong fact is worse than an absent one.
//!
//! So the clock rules are gated on [`FactInput::timezone`], an IANA zone
//! from `UserPreferences`. With no zone stored they simply do not run, and
//! every remaining rule is timezone-independent. A name rather than a fixed
//! offset because an offset is wrong by an hour for half the year wherever
//! DST applies — precisely the boundary these facts sit on.

use chrono::{DateTime, Duration, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Trailing window used by every period-over-period fact. Matches the
/// dashboard's "recent" intuition without being so short that a quiet
/// fortnight erases the comparison.
pub const RECENT_DAYS: i64 = 30;
/// Window for the cadence fact — long enough that a busy week doesn't read
/// as a habit.
pub const CADENCE_DAYS: i64 = 90;
/// Facts returned at most, after ranking.
pub const MAX_FACTS: usize = 3;

/// What a fact's numbers are measured over. Per-fact, NOT per-request:
/// applying a dashboard range to a lifetime fact makes it quietly wrong at
/// 24h — the same defect class as the commerce and corridor range bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FactScope {
    /// Everything we have ever seen for this player.
    Lifetime,
    /// A trailing window, in days.
    Window { days: i64 },
}

/// How to render a value. The engine computes numbers; the surface renders
/// them. Keeping the unit on the evidence stops the web layer guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FactUnit {
    Seconds,
    Count,
    Days,
    /// A ratio already expressed as a multiple (1.8 → "1.8x").
    Multiple,
}

/// The arithmetic behind a claim. Always carries its comparison and the
/// sample it rests on, so a headline can be audited rather than trusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FactEvidence {
    pub value: f64,
    /// What `value` is being compared against. Never optional — see the
    /// module docs.
    pub baseline: f64,
    /// How many observations the claim rests on. Gating lives in the engine,
    /// not in copy: this is what stops "100% of your trades happen on
    /// Sundays" from a single trade.
    pub sample_size: i64,
    pub unit: FactUnit,
}

impl FactEvidence {
    /// Signed strength of the claim: how far `value` departs from
    /// `baseline`, as a fraction. A baseline of zero yields 0.0 rather than
    /// an infinity — an undefined comparison is not a strong one.
    pub fn effect_size(&self) -> f64 {
        if self.baseline.abs() < f64::EPSILON {
            return 0.0;
        }
        (self.value / self.baseline - 1.0).abs()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Fact {
    /// Stable catalogue id. Used for rotation and for the surface to key on;
    /// never shown to the player.
    pub id: String,
    pub scope: FactScope,
    /// One sentence, template over computed evidence. Never LLM prose — a
    /// template can be diffed, tested and defended.
    pub headline: String,
    /// The arithmetic in words, so the claim shows its working.
    pub detail: String,
    pub evidence: FactEvidence,
    /// Where the numbers came from, for the "how do you know that?" case.
    pub provenance: String,
}

/// One session, projected to just what the rules need.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionFacts {
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub death_count: i64,
}

impl SessionFacts {
    fn duration_secs(&self) -> f64 {
        let s = (self.ended_at - self.started_at).num_seconds();
        // A clock skew or a bad rollup row must not produce negative
        // playtime that silently cancels out real sessions in a sum.
        s.max(0) as f64
    }
}

/// Everything the engine needs, assembled once. Deliberately one input for
/// the whole catalogue so twenty facts never become twenty queries.
#[derive(Debug, Clone)]
pub struct FactInput {
    pub now: DateTime<Utc>,
    pub sessions: Vec<SessionFacts>,
    /// The player's IANA zone, when they have set one. `None` disables every
    /// clock-time rule rather than falling back to UTC — see the module docs.
    pub timezone: Option<Tz>,
}

/// Minimum sessions before any fact is emitted. Below this the player has
/// not generated enough signal for a pattern to mean anything, and the
/// surface should say so rather than show a thin claim.
pub const MIN_SESSIONS: usize = 8;

/// Derive the full catalogue, unranked. Rules are independent; each either
/// clears its own sample gate and emits, or stays silent.
pub fn derive_facts(input: &FactInput) -> Vec<Fact> {
    if input.sessions.len() < MIN_SESSIONS {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(f) = fact_session_rhythm(input) {
        out.push(f);
    }
    if let Some(f) = fact_playtime_concentration(input) {
        out.push(f);
    }
    if let Some(f) = fact_flight_cadence(input) {
        out.push(f);
    }
    if let Some(f) = fact_weekly_pace(input) {
        out.push(f);
    }
    if let Some(f) = fact_death_tempo(input) {
        out.push(f);
    }
    // Clock rules run only with a stored zone. No zone → no claim, never a
    // UTC guess.
    if let Some(tz) = input.timezone {
        if let Some(f) = fact_night_owl(input, tz) {
            out.push(f);
        }
        if let Some(f) = fact_busiest_weekday(input, tz) {
            out.push(f);
        }
    }
    out
}

/// Rank by effect size weighted by how much sample backs it, then rotate by
/// `(handle, UTC date)` so the selection is stable within a day, explainable
/// when it changes, and surfaces the whole catalogue over time.
pub fn select_facts(mut facts: Vec<Fact>, handle: &str, today: DateTime<Utc>) -> Vec<Fact> {
    if facts.is_empty() {
        return facts;
    }
    facts.sort_by(|a, b| {
        let sa = score(a);
        let sb = score(b);
        // Ties broken by id so ordering is deterministic across runs.
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    let seed = rotation_seed(handle, today);
    let n = facts.len();
    let offset = (seed % n as u64) as usize;
    facts.rotate_left(offset);
    facts.truncate(MAX_FACTS);
    facts
}

fn score(f: &Fact) -> f64 {
    // Confidence saturates: past a decent sample, more observations should
    // not let a weak effect outrank a strong one.
    let confidence = (f.evidence.sample_size as f64 / 30.0).min(1.0);
    f.evidence.effect_size() * confidence
}

/// Deterministic per-player, per-day rotation. FNV-1a over handle + date —
/// no randomness, so the same day always yields the same selection and a
/// changed selection is always explainable.
fn rotation_seed(handle: &str, today: DateTime<Utc>) -> u64 {
    let key = format!("{}:{}", handle.to_lowercase(), today.format("%Y-%m-%d"));
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn fmt_dur(secs: f64) -> String {
    let s = secs.max(0.0) as i64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    if h >= 24 {
        let d = h / 24;
        return format!("{}d {}h", d, h % 24);
    }
    if h > 0 {
        return format!("{h}h {m:02}m");
    }
    format!("{m}m")
}

/// Median vs mean session length. Interesting because the gap between them
/// IS the observation: a median well below the mean means a few long hauls
/// are carrying the average.
fn fact_session_rhythm(input: &FactInput) -> Option<Fact> {
    let mut durations: Vec<f64> = input
        .sessions
        .iter()
        .map(SessionFacts::duration_secs)
        .filter(|d| *d > 0.0)
        .collect();
    if durations.len() < MIN_SESSIONS {
        return None;
    }
    durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = durations.len();
    let median = if n % 2 == 0 {
        (durations[n / 2 - 1] + durations[n / 2]) / 2.0
    } else {
        durations[n / 2]
    };
    let mean = durations.iter().sum::<f64>() / n as f64;
    if mean <= 0.0 {
        return None;
    }
    let headline = if median < mean {
        format!(
            "Half your flights are under {}, but your average is {}",
            fmt_dur(median),
            fmt_dur(mean)
        )
    } else {
        format!("Your flights cluster tightly around {}", fmt_dur(median))
    };
    Some(Fact {
        id: "session_rhythm".into(),
        scope: FactScope::Lifetime,
        headline,
        detail: format!(
            "Median {} vs mean {} across {} sessions — {}",
            fmt_dur(median),
            fmt_dur(mean),
            n,
            if median < mean {
                "a few long hauls pull the average up"
            } else {
                "no single flight dominates"
            }
        ),
        evidence: FactEvidence {
            value: median,
            baseline: mean,
            sample_size: n as i64,
            unit: FactUnit::Seconds,
        },
        provenance: "session start and end times".into(),
    })
}

/// How few sessions it takes to reach half your total flight time, against
/// the even-split expectation. Surfaces burstiness without a superlative.
fn fact_playtime_concentration(input: &FactInput) -> Option<Fact> {
    let mut durations: Vec<f64> = input
        .sessions
        .iter()
        .map(SessionFacts::duration_secs)
        .filter(|d| *d > 0.0)
        .collect();
    if durations.len() < MIN_SESSIONS {
        return None;
    }
    let total: f64 = durations.iter().sum();
    if total <= 0.0 {
        return None;
    }
    durations.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let mut acc = 0.0;
    let mut needed = 0usize;
    for d in &durations {
        acc += d;
        needed += 1;
        if acc >= total / 2.0 {
            break;
        }
    }
    let n = durations.len();
    // Even distribution would need half the sessions to reach half the time.
    let baseline = n as f64 / 2.0;
    Some(Fact {
        id: "playtime_concentration".into(),
        scope: FactScope::Lifetime,
        headline: format!(
            "Half your time in the 'verse came from just {} of {} flights",
            needed, n
        ),
        detail: format!(
            "{} of {} sessions account for half your {} total — an even split would take {:.0}",
            needed,
            n,
            fmt_dur(total),
            baseline
        ),
        evidence: FactEvidence {
            value: needed as f64,
            baseline,
            sample_size: n as i64,
            unit: FactUnit::Count,
        },
        provenance: "session durations, ranked longest first".into(),
    })
}

/// Days flown out of the trailing window. A habit measure, not a total.
fn fact_flight_cadence(input: &FactInput) -> Option<Fact> {
    let cutoff = input.now - Duration::days(CADENCE_DAYS);
    let mut days: Vec<chrono::NaiveDate> = input
        .sessions
        .iter()
        .filter(|s| s.started_at >= cutoff)
        .map(|s| s.started_at.date_naive())
        .collect();
    if days.is_empty() {
        return None;
    }
    days.sort_unstable();
    days.dedup();
    let flown = days.len() as i64;
    // Only count the window that has actually elapsed since the player's
    // first session, so a new pilot isn't scored against 90 days they were
    // never around for.
    let first = input.sessions.iter().map(|s| s.started_at).min()?;
    let elapsed = (input.now - first.max(cutoff)).num_days().max(1);
    let window = elapsed.min(CADENCE_DAYS);
    if window < 14 {
        // Too short a history for "one day in N" to describe a habit.
        return None;
    }
    let ratio = window as f64 / flown.max(1) as f64;
    Some(Fact {
        id: "flight_cadence".into(),
        scope: FactScope::Window { days: window },
        headline: format!("You've flown on {} of the last {} days", flown, window),
        detail: format!(
            "Roughly one day in {:.1} — {} flying days out of {}",
            ratio, flown, window
        ),
        evidence: FactEvidence {
            value: flown as f64,
            baseline: window as f64,
            sample_size: window,
            unit: FactUnit::Days,
        },
        provenance: "distinct calendar days with at least one session".into(),
    })
}

/// Recent weekly flight time against the lifetime weekly average. Honours
/// the house rule that a comparison means period-over-period, not
/// share-of-lifetime.
fn fact_weekly_pace(input: &FactInput) -> Option<Fact> {
    let cutoff = input.now - Duration::days(RECENT_DAYS);
    let first = input.sessions.iter().map(|s| s.started_at).min()?;
    let lifetime_days = (input.now - first).num_days();
    // Needs meaningfully more history than the recent window, or "recent vs
    // lifetime" is comparing a period against itself.
    if lifetime_days < RECENT_DAYS * 2 {
        return None;
    }
    let recent_secs: f64 = input
        .sessions
        .iter()
        .filter(|s| s.started_at >= cutoff)
        .map(SessionFacts::duration_secs)
        .sum();
    let total_secs: f64 = input.sessions.iter().map(SessionFacts::duration_secs).sum();
    if total_secs <= 0.0 {
        return None;
    }
    let recent_weekly = recent_secs / (RECENT_DAYS as f64 / 7.0);
    let lifetime_weekly = total_secs / (lifetime_days as f64 / 7.0);
    if lifetime_weekly <= 0.0 {
        return None;
    }
    let mult = recent_weekly / lifetime_weekly;
    let headline = if mult >= 1.0 {
        format!(
            "You're flying {} a week lately — {:.1}x your usual",
            fmt_dur(recent_weekly),
            mult
        )
    } else {
        format!(
            "You're flying {} a week lately, down from {}",
            fmt_dur(recent_weekly),
            fmt_dur(lifetime_weekly)
        )
    };
    Some(Fact {
        id: "weekly_pace".into(),
        scope: FactScope::Window { days: RECENT_DAYS },
        headline,
        detail: format!(
            "{} in the last {} days vs a lifetime average of {} a week",
            fmt_dur(recent_secs),
            RECENT_DAYS,
            fmt_dur(lifetime_weekly)
        ),
        evidence: FactEvidence {
            value: recent_weekly,
            baseline: lifetime_weekly,
            sample_size: input.sessions.len() as i64,
            unit: FactUnit::Seconds,
        },
        provenance: "session durations in the last 30 days vs all time".into(),
    })
}

/// Recent deaths per hour of flight against the lifetime rate.
fn fact_death_tempo(input: &FactInput) -> Option<Fact> {
    let cutoff = input.now - Duration::days(RECENT_DAYS);
    let total_deaths: i64 = input.sessions.iter().map(|s| s.death_count).sum();
    // Below this, a rate is noise dressed as a trend.
    if total_deaths < 10 {
        return None;
    }
    let total_secs: f64 = input.sessions.iter().map(SessionFacts::duration_secs).sum();
    let recent: Vec<&SessionFacts> = input
        .sessions
        .iter()
        .filter(|s| s.started_at >= cutoff)
        .collect();
    let recent_secs: f64 = recent.iter().map(|s| s.duration_secs()).sum();
    let recent_deaths: i64 = recent.iter().map(|s| s.death_count).sum();
    if total_secs <= 0.0 || recent_secs <= 0.0 || recent_deaths < 3 {
        return None;
    }
    let lifetime_rate = total_deaths as f64 / (total_secs / 3600.0);
    let recent_rate = recent_deaths as f64 / (recent_secs / 3600.0);
    if lifetime_rate <= 0.0 {
        return None;
    }
    let secs_between_recent = 3600.0 / recent_rate;
    let secs_between_life = 3600.0 / lifetime_rate;
    let headline = if recent_rate > lifetime_rate {
        format!(
            "You're dying every {} lately — usually it's every {}",
            fmt_dur(secs_between_recent),
            fmt_dur(secs_between_life)
        )
    } else {
        format!(
            "You're surviving {} between deaths lately, up from {}",
            fmt_dur(secs_between_recent),
            fmt_dur(secs_between_life)
        )
    };
    Some(Fact {
        id: "death_tempo".into(),
        scope: FactScope::Window { days: RECENT_DAYS },
        headline,
        detail: format!(
            "{} deaths in {} recently vs {} deaths in {} all time",
            recent_deaths,
            fmt_dur(recent_secs),
            total_deaths,
            fmt_dur(total_secs)
        ),
        evidence: FactEvidence {
            value: recent_rate,
            baseline: lifetime_rate,
            sample_size: total_deaths,
            unit: FactUnit::Multiple,
        },
        provenance: "deaths per hour of session time, last 30 days vs all time".into(),
    })
}

/// Share of flight time in the late-night block, against the share a
/// uniformly-spread player would put there.
///
/// The baseline is the honest part. 22:00-04:00 is 6 of 24 hours, so an
/// evenly-spread pilot spends 25% of their time there; "31% of your flying
/// is late night" is only interesting relative to that 25%. Quoting the
/// share alone would make every player look like a night owl.
const NIGHT_START_HOUR: u32 = 22;
const NIGHT_END_HOUR: u32 = 4;
/// 22:00-04:00 as a fraction of the day — the uniform-play expectation.
const NIGHT_BASELINE_SHARE: f64 = 6.0 / 24.0;

fn is_night(hour: u32) -> bool {
    // The night block wraps midnight, so it is expressed as the complement
    // of the daytime range rather than as a range of its own.
    !(NIGHT_END_HOUR..NIGHT_START_HOUR).contains(&hour)
}

fn fact_night_owl(input: &FactInput, tz: Tz) -> Option<Fact> {
    // Attribute each whole hour of a session to the local hour it fell in,
    // rather than bucketing the whole session by its start. A five-hour
    // flight starting at 20:00 is mostly night; counting it as evening
    // would understate exactly the players this fact is about.
    let mut night_secs = 0.0f64;
    let mut total_secs = 0.0f64;
    for s in &input.sessions {
        let dur = s.duration_secs();
        if dur <= 0.0 {
            continue;
        }
        let mut cursor = s.started_at;
        let mut remaining = dur;
        while remaining > 0.0 {
            let local = cursor.with_timezone(&tz);
            // Seconds left in this local hour, capped by what remains.
            let into_hour = (local.minute() * 60 + local.second()) as f64;
            let slice = (3600.0 - into_hour).min(remaining);
            if is_night(local.hour()) {
                night_secs += slice;
            }
            total_secs += slice;
            remaining -= slice;
            cursor += Duration::seconds(slice.max(1.0) as i64);
        }
    }
    if total_secs <= 0.0 {
        return None;
    }
    let share = night_secs / total_secs;
    let mult = share / NIGHT_BASELINE_SHARE;
    // Only worth saying when it departs from uniform in either direction.
    if (mult - 1.0).abs() < 0.25 {
        return None;
    }
    let headline = if mult > 1.0 {
        format!(
            "{:.0}% of your flying is after {}:00 local",
            share * 100.0,
            NIGHT_START_HOUR
        )
    } else {
        format!(
            "You almost never fly late — just {:.0}% after {}:00 local",
            share * 100.0,
            NIGHT_START_HOUR
        )
    };
    Some(Fact {
        id: "night_owl".into(),
        scope: FactScope::Lifetime,
        headline,
        detail: format!(
            "{} of your {} falls between {}:00 and {:02}:00 in {} — spread evenly it would be 25%",
            fmt_dur(night_secs),
            fmt_dur(total_secs),
            NIGHT_START_HOUR,
            NIGHT_END_HOUR,
            tz.name()
        ),
        evidence: FactEvidence {
            value: share,
            baseline: NIGHT_BASELINE_SHARE,
            sample_size: input.sessions.len() as i64,
            unit: FactUnit::Multiple,
        },
        provenance: format!("session hours attributed to local time in {}", tz.name()),
    })
}

/// Busiest local weekday against an average day.
fn fact_busiest_weekday(input: &FactInput, tz: Tz) -> Option<Fact> {
    use chrono::Datelike;
    let mut by_day = [0.0f64; 7];
    for s in &input.sessions {
        let dur = s.duration_secs();
        if dur <= 0.0 {
            continue;
        }
        // Bucketed by LOCAL start day. A session spanning midnight is
        // credited to the day it began, which is how a player thinks of
        // "Friday night" even when it runs past 00:00.
        let local = s.started_at.with_timezone(&tz);
        by_day[local.weekday().num_days_from_monday() as usize] += dur;
    }
    let total: f64 = by_day.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let (idx, best) = by_day
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    let average = total / 7.0;
    if average <= 0.0 {
        return None;
    }
    let mult = best / average;
    // A genuinely even spread has no busiest day worth naming.
    if mult < 1.5 {
        return None;
    }
    const NAMES: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    Some(Fact {
        id: "busiest_weekday".into(),
        scope: FactScope::Lifetime,
        headline: format!("{}s are your day — {:.1}x an average day", NAMES[idx], mult),
        detail: format!(
            "{} on {}s vs {} on an average day, local time in {}",
            fmt_dur(*best),
            NAMES[idx],
            fmt_dur(average),
            tz.name()
        ),
        evidence: FactEvidence {
            value: *best,
            baseline: average,
            sample_size: input.sessions.len() as i64,
            unit: FactUnit::Seconds,
        },
        provenance: format!("session start days in local time ({})", tz.name()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const NOW: &str = "2026-08-07T12:00:00Z";

    /// `n` sessions of `mins` each, one per day counting back from `days_ago`.
    fn sessions(n: usize, mins: i64, days_ago_start: i64, deaths: i64) -> Vec<SessionFacts> {
        (0..n)
            .map(|i| {
                let start = t(NOW) - Duration::days(days_ago_start - i as i64);
                SessionFacts {
                    started_at: start,
                    ended_at: start + Duration::minutes(mins),
                    death_count: deaths,
                }
            })
            .collect()
    }

    fn input(sessions: Vec<SessionFacts>) -> FactInput {
        FactInput {
            now: t(NOW),
            sessions,
            timezone: None,
        }
    }

    fn input_tz(sessions: Vec<SessionFacts>, tz: &str) -> FactInput {
        FactInput {
            now: t(NOW),
            sessions,
            timezone: Some(tz.parse().expect("known zone")),
        }
    }

    /// `n` sessions all starting at the given LOCAL hour in `tz`, one a day.
    fn sessions_at_local_hour(n: usize, tz: &str, hour: u32, mins: i64) -> Vec<SessionFacts> {
        use chrono::{Datelike, TimeZone};
        let zone: chrono_tz::Tz = tz.parse().unwrap();
        (0..n)
            .map(|i| {
                let day = (t(NOW) - Duration::days(120 - i as i64))
                    .with_timezone(&zone)
                    .date_naive();
                let local = zone
                    .with_ymd_and_hms(day.year(), day.month(), day.day(), hour, 0, 0)
                    .single()
                    .expect("unambiguous local time");
                let start = local.with_timezone(&Utc);
                SessionFacts {
                    started_at: start,
                    ended_at: start + Duration::minutes(mins),
                    death_count: 0,
                }
            })
            .collect()
    }

    #[test]
    fn a_thin_history_yields_no_facts_at_all() {
        // The surface must say "not enough flight time yet" rather than
        // showing a claim built on three sessions.
        let facts = derive_facts(&input(sessions(MIN_SESSIONS - 1, 60, 20, 0)));
        assert_eq!(facts, Vec::new());
    }

    #[test]
    fn every_fact_carries_a_baseline_and_a_sample() {
        // The structural invariant: no fact is a bare number.
        let facts = derive_facts(&input(sessions(60, 90, 120, 1)));
        assert!(!facts.is_empty(), "expected facts: {facts:?}");
        for f in &facts {
            assert!(f.evidence.sample_size > 0, "{} has no sample", f.id);
            assert!(
                f.evidence.baseline.is_finite(),
                "{} has a non-finite baseline",
                f.id
            );
            assert!(!f.headline.is_empty());
            assert!(!f.provenance.is_empty(), "{} has no provenance", f.id);
        }
    }

    #[test]
    fn concentration_spots_a_few_long_hauls_carrying_the_total() {
        // 19 short flights plus one enormous one: half the total time should
        // come from a single session, far under the even-split baseline.
        let mut s = sessions(19, 10, 40, 0);
        let start = t(NOW) - Duration::days(5);
        s.push(SessionFacts {
            started_at: start,
            ended_at: start + Duration::hours(10),
            death_count: 0,
        });

        let f = derive_facts(&input(s))
            .into_iter()
            .find(|f| f.id == "playtime_concentration")
            .expect("concentration fact");

        assert_eq!(f.evidence.value, 1.0, "one session should carry half");
        assert_eq!(f.evidence.baseline, 10.0, "even split would take 10 of 20");
        assert!(f.headline.contains("just 1 of 20"));
    }

    #[test]
    fn evenly_sized_flights_sit_at_the_baseline() {
        // The same rule must NOT cry burstiness when there is none.
        let f = derive_facts(&input(sessions(20, 60, 40, 0)))
            .into_iter()
            .find(|f| f.id == "playtime_concentration")
            .expect("concentration fact");

        assert_eq!(f.evidence.baseline, 10.0);
        assert_eq!(f.evidence.value, 10.0, "even split needs half the sessions");
        assert_eq!(f.evidence.effect_size(), 0.0);
    }

    #[test]
    fn weekly_pace_needs_more_history_than_the_window_it_compares() {
        // 20 days of history cannot support "last 30 days vs lifetime" —
        // that compares a period against itself.
        let facts = derive_facts(&input(sessions(20, 60, 20, 0)));
        assert!(
            !facts.iter().any(|f| f.id == "weekly_pace"),
            "pace must stay silent on a short history"
        );
    }

    #[test]
    fn weekly_pace_reports_a_recent_surge_against_lifetime() {
        // Sparse for 200 days, then dense in the last 30.
        let mut s: Vec<SessionFacts> = (0..20)
            .map(|i| {
                let start = t(NOW) - Duration::days(200 - i * 8);
                SessionFacts {
                    started_at: start,
                    ended_at: start + Duration::minutes(30),
                    death_count: 0,
                }
            })
            .collect();
        s.extend(sessions(25, 180, 28, 0));

        let f = derive_facts(&input(s))
            .into_iter()
            .find(|f| f.id == "weekly_pace")
            .expect("pace fact");

        assert!(
            f.evidence.value > f.evidence.baseline,
            "recent pace {} should exceed lifetime {}",
            f.evidence.value,
            f.evidence.baseline
        );
        assert!(f.headline.contains("your usual"), "got: {}", f.headline);
    }

    #[test]
    fn death_tempo_stays_silent_below_its_sample_floor() {
        // Nine deaths is not a rate. Without this gate the fact would
        // announce a trend from noise.
        let facts = derive_facts(&input(sessions(30, 60, 200, 0)));
        assert!(!facts.iter().any(|f| f.id == "death_tempo"));

        let mut s = sessions(30, 60, 200, 0);
        s[0].death_count = 9;
        let facts = derive_facts(&input(s));
        assert!(!facts.iter().any(|f| f.id == "death_tempo"));
    }

    #[test]
    fn cadence_scores_against_elapsed_history_not_the_full_window() {
        // A pilot 20 days old who flew 10 days is "10 of 20", never
        // "10 of 90" — which would read as a lapsed player.
        let f = derive_facts(&input(sessions(10, 60, 20, 0)))
            .into_iter()
            .find(|f| f.id == "flight_cadence")
            .expect("cadence fact");

        assert!(
            f.evidence.baseline <= 21.0,
            "baseline {} should track elapsed history",
            f.evidence.baseline
        );
        assert!(f.headline.contains("10 of"));
    }

    #[test]
    fn a_negative_duration_row_cannot_cancel_out_real_playtime() {
        // Clock skew or a bad rollup row must degrade to zero, not subtract.
        let mut s = sessions(10, 60, 30, 0);
        s[0].ended_at = s[0].started_at - Duration::hours(5);

        let facts = derive_facts(&input(s));
        let c = facts
            .iter()
            .find(|f| f.id == "playtime_concentration")
            .expect("concentration fact");
        assert!(c.evidence.value > 0.0);
        assert!(c.evidence.value.is_finite());
    }

    #[test]
    fn effect_size_of_a_zero_baseline_is_zero_not_infinity() {
        let e = FactEvidence {
            value: 5.0,
            baseline: 0.0,
            sample_size: 10,
            unit: FactUnit::Count,
        };
        assert_eq!(e.effect_size(), 0.0);
    }

    #[test]
    fn selection_is_stable_within_a_day_and_moves_between_days() {
        let facts = derive_facts(&input(sessions(60, 90, 200, 1)));
        assert!(facts.len() >= 2, "need a pool to rotate: {facts:?}");

        let a = select_facts(facts.clone(), "nigel", t("2026-08-07T01:00:00Z"));
        let b = select_facts(facts.clone(), "nigel", t("2026-08-07T23:00:00Z"));
        assert_eq!(a, b, "same day must yield the same selection");

        // Across the catalogue, some day must differ — otherwise rotation is
        // decorative and the tail of the catalogue never surfaces.
        let differs = (0..14).any(|d| {
            let day = t("2026-08-07T12:00:00Z") + Duration::days(d);
            select_facts(facts.clone(), "nigel", day) != a
        });
        assert!(differs, "rotation never changed the selection");
    }

    #[test]
    fn selection_returns_at_most_the_display_cap() {
        let facts = derive_facts(&input(sessions(60, 90, 200, 1)));
        let picked = select_facts(facts, "nigel", t(NOW));
        assert!(picked.len() <= MAX_FACTS);
    }

    #[test]
    fn two_players_on_the_same_day_can_see_different_facts() {
        let facts = derive_facts(&input(sessions(60, 90, 200, 1)));
        let seeds: Vec<u64> = ["nigel", "someone-else", "third"]
            .iter()
            .map(|h| rotation_seed(h, t(NOW)))
            .collect();
        assert!(
            seeds.iter().collect::<std::collections::HashSet<_>>().len() > 1,
            "handle must affect the seed"
        );
        assert!(!facts.is_empty());
    }

    #[test]
    fn rotation_seed_ignores_handle_case() {
        // Handle casing has bitten event queries before; the seed must not
        // make a re-paired user's facts jump around.
        assert_eq!(
            rotation_seed("Nigel", t(NOW)),
            rotation_seed("nigel", t(NOW))
        );
    }

    #[test]
    fn duration_formatting_reads_naturally_at_each_scale() {
        assert_eq!(fmt_dur(90.0), "1m");
        assert_eq!(fmt_dur(3600.0), "1h 00m");
        assert_eq!(fmt_dur(3600.0 * 5.5), "5h 30m");
        assert_eq!(fmt_dur(3600.0 * 30.0), "1d 6h");
        assert_eq!(fmt_dur(-5.0), "0m");
    }

    #[test]
    fn night_owl_fires_for_a_player_who_flies_late_in_their_own_zone() {
        // 40 flights starting 23:00 local, 2h each — squarely in the
        // 22:00-04:00 block.
        let facts = derive_facts(&input_tz(
            sessions_at_local_hour(40, "Europe/London", 23, 120),
            "Europe/London",
        ));
        let f = facts
            .iter()
            .find(|f| f.id == "night_owl")
            .expect("night owl fact");

        assert!(f.evidence.value > 0.9, "share was {}", f.evidence.value);
        assert_eq!(f.evidence.baseline, NIGHT_BASELINE_SHARE);
        assert!(f.headline.contains("after 22:00 local"));
        assert!(f.detail.contains("Europe/London"));
    }

    #[test]
    fn night_owl_stays_silent_for_a_daytime_player() {
        // The rule must not flatter everyone: 14:00 flights are not night.
        let facts = derive_facts(&input_tz(
            sessions_at_local_hour(40, "Europe/London", 14, 120),
            "Europe/London",
        ));
        let f = facts.iter().find(|f| f.id == "night_owl");
        // Either absent, or present as the "almost never" reading — never
        // as a night-owl claim.
        if let Some(f) = f {
            assert!(f.evidence.value < NIGHT_BASELINE_SHARE);
            assert!(f.headline.contains("almost never"));
        }
    }

    #[test]
    fn the_same_sessions_read_differently_in_a_different_zone() {
        // THE point of storing a zone. 23:00 in London is 18:00 in New York
        // and 08:00 in Sydney — the same UTC instants, three different
        // answers. A UTC fallback would have shipped one of them to
        // everybody.
        let utc_sessions = sessions_at_local_hour(40, "Europe/London", 23, 120);

        let london = derive_facts(&input_tz(utc_sessions.clone(), "Europe/London"));
        let sydney = derive_facts(&input_tz(utc_sessions, "Australia/Sydney"));

        let l = london.iter().find(|f| f.id == "night_owl");
        let s = sydney.iter().find(|f| f.id == "night_owl");
        assert!(l.is_some(), "London should read as night flying");
        match s {
            None => {}
            Some(s) => assert!(
                s.evidence.value < l.unwrap().evidence.value,
                "Sydney share {} should be lower than London {}",
                s.evidence.value,
                l.unwrap().evidence.value
            ),
        }
    }

    #[test]
    fn a_session_spanning_midnight_splits_across_hours() {
        // A flight from 20:00 to 02:00 is 2h evening + 4h night. Bucketing
        // the whole session by its START hour would call it 0% night and
        // understate exactly the players this fact is about.
        let s = sessions_at_local_hour(30, "Europe/London", 20, 360);
        let facts = derive_facts(&input_tz(s, "Europe/London"));
        let f = facts
            .iter()
            .find(|f| f.id == "night_owl")
            .expect("night owl fact");

        // 4 of every 6 hours fall after 22:00.
        assert!(
            (f.evidence.value - 4.0 / 6.0).abs() < 0.05,
            "expected ~0.67 night share, got {}",
            f.evidence.value
        );
    }

    #[test]
    fn busiest_weekday_needs_a_real_skew_not_just_a_maximum() {
        // Evenly spread play has a maximum day, but naming it would be
        // noise dressed as insight.
        let even = derive_facts(&input_tz(sessions(70, 60, 100, 0), "Europe/London"));
        assert!(
            !even.iter().any(|f| f.id == "busiest_weekday"),
            "even spread must not name a busiest day"
        );
    }

    #[test]
    fn clock_facts_join_the_rotation_pool_once_a_zone_exists() {
        // The catalogue should genuinely grow, not just gain dead rules.
        let without = derive_facts(&input(sessions_at_local_hour(40, "Europe/London", 23, 120)));
        let with = derive_facts(&input_tz(
            sessions_at_local_hour(40, "Europe/London", 23, 120),
            "Europe/London",
        ));
        assert!(
            with.len() > without.len(),
            "a stored zone should unlock additional facts ({} vs {})",
            with.len(),
            without.len()
        );
    }

    #[test]
    fn no_clock_time_claim_without_a_stored_timezone() {
        // The invariant this feature rests on. With no zone, a clock claim
        // is wrong for most of the planet, so none may be emitted — and
        // falling back to UTC is NOT an acceptable substitute.
        let facts = derive_facts(&input(sessions(60, 90, 200, 1)));
        assert!(
            !facts
                .iter()
                .any(|f| f.id == "night_owl" || f.id == "busiest_weekday"),
            "clock facts must not run without a timezone"
        );
        for f in &facts {
            let text = format!("{} {}", f.headline, f.detail).to_lowercase();
            for banned in [
                "monday",
                "tuesday",
                "wednesday",
                "thursday",
                "friday",
                "saturday",
                "sunday",
                "morning",
                "evening",
                "midnight",
                "am",
                "pm",
            ] {
                assert!(
                    !text
                        .split_whitespace()
                        .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == banned),
                    "{} claims a clock-time pattern ({banned}) with no stored timezone: {text}",
                    f.id
                );
            }
        }
    }
}
