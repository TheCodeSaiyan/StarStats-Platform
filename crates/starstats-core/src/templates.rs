//! Multi-line "ritual" template + burst matching for Game.log.
//!
//! Some Star Citizen log events are emitted as a *group* of lines that
//! together signal one higher-order activity. This module gives us two
//! primitives for recognising such groups deterministically:
//!
//! * [`EventTemplate`] — fixed-cardinality sequence matching. Use when
//!   a ritual has a known number of distinct steps (e.g. session-start
//!   ProcessInit → LegacyLogin → JoinPu → SeedSolarSystem). Surfaces
//!   drift via [`TemplateHit::missing_steps`] when CIG renames one of
//!   the steps mid-patch.
//!
//! * [`BurstRule`] — variable-cardinality burst clustering. Use when a
//!   ritual is "one anchor line followed by N more of the same kind"
//!   (e.g. the spawn-restore burst of 20+ `<AttachmentReceived>` lines,
//!   the planet-terrain-load shower, jurisdiction HUD-banner stutters).
//!   Collapses spammy per-line events into one summary [`BurstHit`].
//!
//! ## Architecture
//!
//! Pure functions, no state. The caller (typically the tray's gamelog
//! ingest loop) maintains a rolling buffer of recent [`LogLine`]s and
//! invokes [`match_templates`] / [`detect_bursts`] on the window.
//! Multiple rules of either kind may match the same window.
//!
//! Semantic interpretation — turning a [`TemplateHit`] or [`BurstHit`]
//! into a typed [`crate::GameEvent`] — is the *caller's* job. This
//! module only describes structural fingerprints; it doesn't carry
//! payload-shaping logic or own a wire-format dependency.

use crate::parser::LogLine;
use serde::{Deserialize, Serialize};

// =====================================================================
// Step / fingerprint vocabulary, shared by templates and bursts
// =====================================================================

/// Strategies for matching one [`LogLine`] to one fingerprint slot.
///
/// Used by both [`TemplateStep`] and [`BurstRule`]. The variants are
/// ordered roughly from rigid to permissive; pick the most rigid one
/// that fits — that maximises the value of drift detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepMatch {
    /// Match if `LogLine.event_name == name`. Use for events with
    /// stable `<EventName>` shells.
    EventName { name: String },
    /// Match if `LogLine.body.starts_with(prefix)`. Use for inline
    /// events anchored on a stable body prefix (e.g. `body_*`
    /// corpse/spawn attachments).
    BodyStartsWith { prefix: String },
    /// Match if `LogLine.body.contains(needle)`. Broader; higher
    /// false-positive risk.
    BodyContains { needle: String },
    /// Wildcard — matches any structural-parseable line. Useful as an
    /// explicit "skip up to here" anchor in templates.
    Any,
}

/// Evaluate one [`StepMatch`] against one [`LogLine`], including
/// optional tag constraints. Tags must ALL be present on the line.
fn line_matches(line: &LogLine<'_>, mat: &StepMatch, tags: &[String]) -> bool {
    let body_match = match mat {
        StepMatch::EventName { name } => line.event_name == Some(name.as_str()),
        StepMatch::BodyStartsWith { prefix } => line.body.starts_with(prefix.as_str()),
        StepMatch::BodyContains { needle } => line.body.contains(needle.as_str()),
        StepMatch::Any => true,
    };
    if !body_match {
        return false;
    }
    for tag in tags {
        if !line.tags.contains(&tag.as_str()) {
            return false;
        }
    }
    true
}

// =====================================================================
// Fixed-sequence templates (drift detection)
// =====================================================================

/// One ritual definition. Steps are walked in order against a sliding
/// window of `LogLine`s; up to `max_slack` interleaved lines may
/// appear cumulatively between matched steps.
///
/// Time-bounding (e.g. "the entire ritual must complete inside 8s") is
/// the caller's responsibility — they control window size. The matcher
/// stays timestamp-agnostic so it's trivially testable on synthetic
/// fixtures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventTemplate {
    /// Stable id, used to identify which ritual fired.
    pub id: String,
    /// Ordered steps. Position matters.
    pub steps: Vec<TemplateStep>,
    /// Cumulative interleaved lines tolerated across the whole walk.
    /// `0` = strict adjacency.
    pub max_slack: usize,
    /// Minimum number of *required* (non-optional) steps that must
    /// match for the template to fire.
    pub min_match_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateStep {
    #[serde(rename = "match")]
    pub r#match: StepMatch,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional steps don't count toward `min_match_count` and don't
    /// appear in `missing_steps` when skipped.
    #[serde(default)]
    pub optional: bool,
    /// Free-form label, surfaced in drift reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateHit {
    pub template_id: String,
    /// How many of `template.steps` matched a line (optional + required).
    pub matched_count: usize,
    pub total_steps: usize,
    /// `required_matched / required_total`, capped at 1.0. Templates
    /// with zero required steps return 1.0.
    pub confidence: f32,
    /// Window indices of each matched step, in step order. `None` for
    /// skipped optional steps (or unmatched optional/required slots).
    pub captured_indices: Vec<Option<usize>>,
    /// Step indices that were required but didn't match — drives
    /// drift reporting.
    pub missing_steps: Vec<usize>,
}

/// Match a window against a list of templates. Returns every hit
/// clearing its `min_match_count`. Pure, no state.
pub fn match_templates(window: &[LogLine<'_>], templates: &[EventTemplate]) -> Vec<TemplateHit> {
    templates
        .iter()
        .filter_map(|tpl| match_one_template(window, tpl))
        .collect()
}

fn match_one_template(window: &[LogLine<'_>], tpl: &EventTemplate) -> Option<TemplateHit> {
    if window.is_empty() || tpl.steps.is_empty() {
        return None;
    }

    let mut window_pos = 0;
    let mut slack_used = 0;
    let mut captured: Vec<Option<usize>> = Vec::with_capacity(tpl.steps.len());
    let mut missing: Vec<usize> = Vec::new();

    for (step_idx, step) in tpl.steps.iter().enumerate() {
        let mut found_at: Option<usize> = None;
        let mut local_skip = 0usize;
        for (offset, line) in window[window_pos..].iter().enumerate() {
            if line_matches(line, &step.r#match, &step.tags) {
                found_at = Some(window_pos + offset);
                local_skip = offset;
                break;
            }
            // Stop once the next iteration would put us over budget.
            if slack_used + offset >= tpl.max_slack {
                break;
            }
        }
        match found_at {
            Some(idx) => {
                captured.push(Some(idx));
                slack_used += local_skip;
                window_pos = idx + 1;
            }
            None => {
                captured.push(None);
                if !step.optional {
                    missing.push(step_idx);
                }
            }
        }
    }

    let required_total = tpl.steps.iter().filter(|s| !s.optional).count();
    let required_matched = captured
        .iter()
        .zip(tpl.steps.iter())
        .filter(|(c, s)| c.is_some() && !s.optional)
        .count();

    if required_matched < tpl.min_match_count {
        return None;
    }

    let confidence = if required_total == 0 {
        1.0
    } else {
        (required_matched.min(required_total) as f32) / (required_total as f32)
    };
    let matched_count = captured.iter().filter(|c| c.is_some()).count();

    Some(TemplateHit {
        template_id: tpl.id.clone(),
        matched_count,
        total_steps: tpl.steps.len(),
        confidence,
        captured_indices: captured,
        missing_steps: missing,
    })
}

// =====================================================================
// Variable-cardinality burst clustering (spam reduction)
// =====================================================================

/// One burst-detection rule. Matches a contiguous (modulo tolerated
/// gaps) run of N+ lines that share a common fingerprint, signalling
/// a single semantic activity worth collapsing into one event.
///
/// Use cases (from observed Game.log spam): the spawn-restore burst
/// of `<AttachmentReceived>`, the `<StatObjLoad>` shower during
/// planet entry, jurisdiction `<SHUDEvent_OnNotification>` stutters,
/// and `<VehicleStowed>` runs when entering a hangar.
///
/// `anchor` and `member` are usually the same matcher (the burst is
/// homogeneous — every line is the same kind of event), but allowing
/// them to differ supports patterns like "one body_* anchor followed
/// by N+ AttachmentReceived without body_ requirement".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BurstRule {
    pub id: String,
    /// First line of the burst — the matcher that decides "this is a
    /// burst boundary, start counting members from here".
    pub anchor: StepMatch,
    /// Subsequent lines that count toward the burst membership.
    /// Typically the same shape as `anchor`.
    pub member: StepMatch,
    /// Required tags on both anchor and member lines.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Minimum total lines (anchor + members) before the rule fires.
    /// `1` means even a lone anchor counts; for spam-reduction set
    /// to `≥ 3` so short, legitimate sequences aren't clobbered.
    pub min_burst_size: usize,
    /// Maximum non-matching lines tolerated between consecutive
    /// burst members before the burst is considered ended. `0` means
    /// any non-member breaks the burst.
    pub max_member_gap: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BurstHit {
    pub rule_id: String,
    /// Window index of the anchor line.
    pub start_index: usize,
    /// Window index of the LAST member line in the burst.
    pub end_index: usize,
    /// Total lines in the burst (anchor + members). Always
    /// `>= min_burst_size`.
    pub size: usize,
    /// All window indices that participated in the burst, in order.
    /// Useful for downstream suppression of the per-line events.
    pub member_indices: Vec<usize>,
}

/// Scan a window for bursts. Returns one [`BurstHit`] per burst. Bursts
/// don't overlap — once a member is consumed by a burst, subsequent
/// rules don't re-claim it (first-rule-wins). Within a single rule,
/// disjoint bursts in the same window all fire.
pub fn detect_bursts(window: &[LogLine<'_>], rules: &[BurstRule]) -> Vec<BurstHit> {
    detect_bursts_inner(window, rules, None)
}

/// Like [`detect_bursts`], but a candidate member whose timestamp is more
/// than `max_member_gap_secs` away (either direction) from the previously
/// accepted member ends the burst.
///
/// Used ONLY by the tray Re-parse path. Its scan window is ordered by
/// `source_offset`, which resets to 0 on every Game.log rotation — so
/// attachment runs from *different* sessions/files at overlapping offsets
/// land adjacent in the window. Without a wall-clock ceiling
/// [`detect_bursts`] welds them into one `BurstSummary` and then deletes
/// the (cross-session) member rows. The live-drain path keeps calling
/// [`detect_bursts`] with no ceiling so its documented timestamp-agnostic
/// behaviour is untouched. Unparseable timestamps are treated as "no gap"
/// (permissive) — matching the live path rather than over-splitting.
pub fn detect_bursts_with_time_gap(
    window: &[LogLine<'_>],
    rules: &[BurstRule],
    max_member_gap_secs: i64,
) -> Vec<BurstHit> {
    detect_bursts_inner(window, rules, Some(max_member_gap_secs))
}

/// True when both timestamps parse as RFC3339 and differ by more than
/// `max_secs` in either direction. Unparseable input returns `false` so a
/// malformed line never forces a burst split.
fn member_time_gap_exceeds(prev_ts: &str, cur_ts: &str, max_secs: i64) -> bool {
    match (
        chrono::DateTime::parse_from_rfc3339(prev_ts),
        chrono::DateTime::parse_from_rfc3339(cur_ts),
    ) {
        (Ok(a), Ok(b)) => (b - a).num_seconds().abs() > max_secs,
        _ => false,
    }
}

fn detect_bursts_inner(
    window: &[LogLine<'_>],
    rules: &[BurstRule],
    max_member_gap_secs: Option<i64>,
) -> Vec<BurstHit> {
    if window.is_empty() {
        return Vec::new();
    }
    let mut consumed: Vec<bool> = vec![false; window.len()];
    let mut hits: Vec<BurstHit> = Vec::new();

    for rule in rules {
        let mut i = 0;
        while i < window.len() {
            if consumed[i] {
                i += 1;
                continue;
            }
            // Anchor candidate?
            if !line_matches(&window[i], &rule.anchor, &rule.tags) {
                i += 1;
                continue;
            }
            // Found an anchor; walk forward collecting members until
            // we hit `max_member_gap + 1` consecutive non-members or
            // run off the end of the window.
            let mut members = vec![i];
            let mut gap = 0usize;
            let mut j = i + 1;
            while j < window.len() {
                if consumed[j] {
                    // A previous burst already claimed this slot;
                    // treat it like a non-member and respect the gap
                    // budget.
                    gap += 1;
                    if gap > rule.max_member_gap {
                        break;
                    }
                    j += 1;
                    continue;
                }
                if line_matches(&window[j], &rule.member, &rule.tags) {
                    // Reparse-only wall-clock ceiling: a member too far in
                    // time from the previous accepted member ends the burst
                    // (breaks cross-session welds when the offset-sorted
                    // window interleaves rotated Game.log files).
                    if let Some(max) = max_member_gap_secs {
                        let prev = *members.last().unwrap();
                        if member_time_gap_exceeds(window[prev].timestamp, window[j].timestamp, max)
                        {
                            break;
                        }
                    }
                    members.push(j);
                    gap = 0;
                    j += 1;
                } else {
                    gap += 1;
                    if gap > rule.max_member_gap {
                        break;
                    }
                    j += 1;
                }
            }

            if members.len() >= rule.min_burst_size {
                for &idx in &members {
                    consumed[idx] = true;
                }
                hits.push(BurstHit {
                    rule_id: rule.id.clone(),
                    start_index: members[0],
                    end_index: *members.last().unwrap(),
                    size: members.len(),
                    member_indices: members,
                });
                // Resume scanning past the burst.
                i = j;
            } else {
                // Anchor didn't grow into a burst — try the next slot.
                i += 1;
            }
        }
    }

    // Sort by start index for downstream determinism (rules iterate in
    // declaration order, but a later rule's burst may start earlier
    // than a prior rule's).
    hits.sort_by_key(|h| h.start_index);
    hits
}

// =====================================================================
// Loadout item categoriser
// =====================================================================

/// Categorise a loadout item id into one of five buckets used by the
/// web `loadout` widget.
///
/// ## Buckets
///
/// | Bucket         | Returned string   | Examples                                  |
/// |----------------|-------------------|-------------------------------------------|
/// | Weapons        | `"weapons"`       | rifles, pistols, SMGs, LMGs, shotguns     |
/// | Armour         | `"armor"`         | helmets, torsos, undersuits, backpacks    |
/// | Attachments    | `"attachments"`   | scopes, grips, barrels, suppressors       |
/// | Consumables    | `"consumables"`   | medpens, adren pens, grenades             |
/// | Safety valve   | `"unknown"`       | anything not matched by the patterns below|
///
/// ## Why `"unknown"` is intentional
///
/// Star Citizen item IDs don't follow a single rigid schema — manufacturer
/// prefixes, legacy names, and future categories can all produce IDs that
/// don't fit the current pattern set. Returning `"unknown"` is safer than
/// misclassifying an item into the wrong bucket. The web `loadout` widget
/// renders all five buckets in a fixed order; `"unknown"` appears last.
///
/// ## How to add new patterns
///
/// 1. Convert the new pattern to lowercase (the function lowercases `item_id`
///    before matching, so all literals here must be lowercase).
/// 2. **Prefer `contains("_foo_")` (leading-underscore anchors) over
///    `starts_with("foo_")`** — manufacturer-prefixed IDs such as
///    `klwe_pistol_01` are caught by `_pistol_` but not by `starts_with`.
///    Use `starts_with` only when the pattern genuinely only ever appears
///    at the start (e.g. the `weapon_` / `armor_` canonical prefixes).
/// 3. Add a test in `loadout_categorise_tests` for every new pattern, using
///    a realistic-looking item ID. This keeps regressions visible.
/// 4. Keep the five-bucket output constant — do NOT add new category strings.
///    The web widget renders buckets in a fixed order and will silently drop
///    anything outside the five names.
pub fn categorise_loadout_item(item_id: &str) -> &'static str {
    let lower = item_id.to_lowercase();

    // ------------------------------------------------------------------
    // Weapons — firearms, energy weapons, and manufacturer combos
    // ------------------------------------------------------------------
    if lower.starts_with("weapon_")
        || lower.starts_with("wpn_")
        // Weapon type substrings (underscore-anchored to avoid false matches)
        || lower.contains("_rifle_")
        || lower.contains("_pistol_")
        || lower.contains("_ar_")       // assault rifle
        || lower.contains("_lmg_")      // light machine gun
        || lower.contains("_smg_")      // sub-machine gun
        || lower.contains("_dmr_")      // designated marksman rifle
        || lower.contains("_sniper_")
        || lower.contains("_shotgun_")
        || lower.contains("_ballistic_")
        || lower.contains("_energy_")   // energy weapons
        // Manufacturer-prefixed weapon items
        || lower.starts_with("ksar_")   // Klaus & Werner assault rifles
        || lower.starts_with("gmni_")   // Gemini weapon items
        // Weapon model substrings — specific RSI/manufacturer identifiers
        || lower.contains("_p4ar")
    // RSI P4-AR carbine (rsi_p4ar_01 etc.)
    {
        return "weapons";
    }

    // ------------------------------------------------------------------
    // Armour — suits, helmets, limb pieces, and carrying gear
    // ------------------------------------------------------------------
    if lower.starts_with("armor_")
        || lower.starts_with("arm_")
        || lower.starts_with("helmet_")  // helmet-first IDs
        || lower.contains("_helmet_")
        || lower.contains("_torso_")
        || lower.contains("_arms_")
        || lower.contains("_legs_")
        || lower.contains("_undersuit_")
        || lower.contains("_backpack_")
        || lower.contains("_chest_")
        || lower.contains("heavy_armor_")
        || lower.contains("medium_armor_")
        || lower.contains("light_armor_")
    {
        return "armor";
    }

    // ------------------------------------------------------------------
    // Attachments — barrel, optic, magazine, and underbarrel mods
    // ------------------------------------------------------------------
    if lower.starts_with("att_")
        || lower.contains("_scope_")
        || lower.contains("_grip_")
        || lower.contains("_magazine_")  // full form; safer than _mag_
        || lower.contains("_sight_")
        || lower.contains("_barrel_")
        || lower.contains("_muzzle_")
        || lower.contains("_suppressor_")
        || lower.contains("_compensator_")
        || lower.contains("_underbarrel_")
    {
        return "attachments";
    }

    // ------------------------------------------------------------------
    // Consumables — medical pens, grenades, and utility throwables
    // ------------------------------------------------------------------
    if lower.starts_with("consum_")
        || lower.contains("_medpen_")
        || lower.contains("_grenade_")
        || lower.contains("_hempen_")       // Hempen medical injector
        || lower.contains("_adrenpen_")     // adrenaline pen
        || lower.contains("_detrapen_")     // detox/trauma pen
        || lower.contains("_oxypen_")       // oxygen pen
        || lower.contains("_frag_")         // fragmentation grenade
        || lower.contains("_flashbang_")
        || lower.contains("_smoke_grenade_")
        || lower.contains("_emp_grenade_")
    {
        return "consumables";
    }

    "unknown"
}

// =====================================================================
// Loadout-restore payload builder
// =====================================================================

/// Build the `kind` and `categories` fields for a `loadout_restore`
/// [`crate::events::BurstSummary`].
///
/// Takes a slice of `item_class` strings extracted from the burst
/// members (one entry per `<AttachmentReceived>` line) and returns a
/// `HashMap` of category → count, keyed by the five buckets produced
/// by [`categorise_loadout_item`].
///
/// Returns `None` when `item_classes` is empty so the caller can
/// skip setting the field on the summary (avoids emitting an empty
/// `categories: {}` object on pathological zero-member bursts).
///
/// # Example
///
/// ```rust
/// use starstats_core::templates::build_loadout_categories;
///
/// let items = ["WEAPON_lr2_p4ar", "ARMOR_pilotsuit_v3", "WEAPON_ballistic_01"];
/// let cats = build_loadout_categories(&items).unwrap();
/// assert_eq!(cats["weapons"], 2);
/// assert_eq!(cats["armor"], 1);
/// ```
pub fn build_loadout_categories(
    item_classes: &[impl AsRef<str>],
) -> Option<std::collections::HashMap<String, u64>> {
    if item_classes.is_empty() {
        return None;
    }
    let mut map: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for ic in item_classes {
        let cat = categorise_loadout_item(ic.as_ref());
        *map.entry(cat.to_string()).or_insert(0) += 1;
    }
    Some(map)
}

/// Build the per-item list for a `loadout_restore` BurstSummary from
/// `(item_class, port)` pairs (one per `<AttachmentReceived>` member).
/// `None` when empty (caller skips setting the field).
pub fn build_loadout_items(pairs: &[(String, String)]) -> Option<Vec<crate::events::LoadoutItem>> {
    if pairs.is_empty() {
        return None;
    }
    Some(
        pairs
            .iter()
            .map(|(class, port)| crate::events::LoadoutItem {
                class: class.clone(),
                port: port.clone(),
                category: categorise_loadout_item(class).to_string(),
            })
            .collect(),
    )
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::structural_parse;

    fn lines(s: &str) -> Vec<LogLine<'_>> {
        s.lines().filter_map(structural_parse).collect()
    }

    fn event_step(name: &str) -> TemplateStep {
        TemplateStep {
            r#match: StepMatch::EventName { name: name.into() },
            tags: vec![],
            optional: false,
            label: None,
        }
    }

    // -----------------------------------------------------------------
    // EventTemplate tests — fixed-sequence ritual matching
    // -----------------------------------------------------------------

    const SESSION_START_FIXTURE: &str = "\
<2026-05-02T21:01:55.000Z> <Init> Process sc-client started (Local: 2f33fc1e. Env: pub-sc-alpha-470). Online: bOnline[1] [System][Initialization]
<2026-05-02T21:02:01.500Z> <Legacy login response> Handle[TheCodeSaiyan] - Time[2026-05-02T21:02:01Z]
<2026-05-02T21:02:03.000Z> <Join PU> address[10.0.0.1] port[64090] shard[shard-eu-1] locationId[loc-stanton]
<2026-05-02T21:02:05.000Z> <Seed Solar System> in Solar System 'Stanton' for shard shard-eu-1 [Team_OnlineTech][Init]
";

    #[test]
    fn template_matches_full_session_start_ritual() {
        let window = lines(SESSION_START_FIXTURE);
        assert_eq!(window.len(), 4, "fixture should produce 4 LogLines");

        let tpl = EventTemplate {
            id: "session_start".into(),
            steps: vec![
                event_step("Init"),
                event_step("Legacy login response"),
                event_step("Join PU"),
                event_step("Seed Solar System"),
            ],
            max_slack: 0,
            min_match_count: 4,
        };
        let hits = match_templates(&window, std::slice::from_ref(&tpl));
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.template_id, "session_start");
        assert_eq!(hit.matched_count, 4);
        assert!((hit.confidence - 1.0).abs() < f32::EPSILON);
        assert_eq!(
            hit.captured_indices,
            vec![Some(0), Some(1), Some(2), Some(3)]
        );
        assert!(hit.missing_steps.is_empty());
    }

    #[test]
    fn template_drift_detection_partial_match() {
        // Step 1 (Legacy login response) is renamed to "Login Response".
        const RENAMED: &str = "\
<2026-05-02T21:01:55.000Z> <Init> Process sc-client started (Local: x. Env: y). Online: bOnline[1] [System]
<2026-05-02T21:02:01.500Z> <Login Response> Handle[X] - Time[Y]
<2026-05-02T21:02:03.000Z> <Join PU> address[10.0.0.1] port[64090] shard[s] locationId[l]
<2026-05-02T21:02:05.000Z> <Seed Solar System> in Solar System 'Stanton' for shard s [Team_OnlineTech]
";
        let window = lines(RENAMED);
        let tpl = EventTemplate {
            id: "session_start".into(),
            steps: vec![
                event_step("Init"),
                event_step("Legacy login response"),
                event_step("Join PU"),
                event_step("Seed Solar System"),
            ],
            // One slack lets the matcher skip over the renamed line
            // when looking for "Join PU".
            max_slack: 1,
            min_match_count: 3,
        };
        let hits = match_templates(&window, std::slice::from_ref(&tpl));
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.matched_count, 3);
        assert!((hit.confidence - 0.75).abs() < 0.01);
        assert_eq!(hit.missing_steps, vec![1]);
        assert_eq!(hit.captured_indices, vec![Some(0), None, Some(2), Some(3)]);
    }

    #[test]
    fn template_min_match_count_gates_false_positives() {
        const ONE_STEP: &str = "\
<2026-05-02T21:01:55.000Z> <Init> Process sc-client started (Local: a. Env: b). Online: bOnline[1] [System]
<2026-05-02T21:01:56.000Z> <Unrelated> some other line [Team_X]
";
        let window = lines(ONE_STEP);
        let tpl = EventTemplate {
            id: "session_start".into(),
            steps: vec![event_step("Init"), event_step("Legacy login response")],
            max_slack: 0,
            min_match_count: 2,
        };
        assert!(match_templates(&window, std::slice::from_ref(&tpl)).is_empty());
    }

    #[test]
    fn template_optional_step_not_required_for_hit() {
        const ONLY_REQUIRED: &str = "\
<2026-05-02T21:01:55.000Z> <Init> body [System]
<2026-05-02T21:02:01.500Z> <Legacy login response> Handle[X] - Time[Y]
";
        let window = lines(ONLY_REQUIRED);
        let tpl = EventTemplate {
            id: "session_start_with_optional".into(),
            steps: vec![
                event_step("Init"),
                event_step("Legacy login response"),
                TemplateStep {
                    r#match: StepMatch::EventName {
                        name: "Join PU".into(),
                    },
                    tags: vec![],
                    optional: true,
                    label: Some("optional_join".into()),
                },
            ],
            max_slack: 0,
            min_match_count: 2,
        };
        let hits = match_templates(&window, std::slice::from_ref(&tpl));
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.matched_count, 2);
        assert!(hit.missing_steps.is_empty());
        assert!((hit.confidence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn template_slack_budget_caps_interleaving() {
        const TOO_MUCH_NOISE: &str = "\
<T1> <Init> body [System]
<T2> <Noise1> body
<T3> <Noise2> body
<T4> <Noise3> body
<T5> <Legacy login response> body
";
        let window = lines(TOO_MUCH_NOISE);
        let tpl = EventTemplate {
            id: "tight_session_start".into(),
            steps: vec![event_step("Init"), event_step("Legacy login response")],
            max_slack: 2,
            min_match_count: 2,
        };
        assert!(match_templates(&window, std::slice::from_ref(&tpl)).is_empty());
    }

    // -----------------------------------------------------------------
    // BurstRule tests — variable-cardinality clustering
    // -----------------------------------------------------------------

    /// Six-line spawn-restore burst, modelled on the real fixture
    /// at `tests/fixtures/sample_game_log.txt:1272`. All
    /// `<AttachmentReceived>` with `[Inventory]` tag, all stamped at
    /// the same millisecond. The first one carries the `body_*`
    /// attachment that anchors the burst; the rest are armor + weapons.
    const ATTACHMENT_BURST_FIXTURE: &str = "\
<2026-05-02T21:15:03.053Z> <AttachmentReceived> Player[X] Attachment[body_01_noMagicPocket_999, body_01_noMagicPocket, 999] Status[persistent] Port[Body_ItemPort] Elapsed[0.1] [Team_CoreGameplayFeatures][Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> Player[X] Attachment[vgl_undersuit_01_888] Status[persistent] Port[Armor_Undersuit] Elapsed[0.0] [Team_CoreGameplayFeatures][Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> Player[X] Attachment[rrs_arms_03_777] Status[persistent] Port[Armor_Arms] Elapsed[0.0] [Team_CoreGameplayFeatures][Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> Player[X] Attachment[rrs_helmet_03_666] Status[persistent] Port[Armor_Helmet] Elapsed[0.0] [Team_CoreGameplayFeatures][Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> Player[X] Attachment[rrs_legs_03_555] Status[persistent] Port[Armor_Legs] Elapsed[0.0] [Team_CoreGameplayFeatures][Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> Player[X] Attachment[klwe_pistol_01_444] Status[persistent] Port[wep_sidearm] Elapsed[0.0] [Team_CoreGameplayFeatures][Inventory]
<2026-05-02T21:15:04.000Z> <SomethingElse> body [Team_X]
";

    #[test]
    fn burst_collapses_attachment_received_spam() {
        let window = lines(ATTACHMENT_BURST_FIXTURE);
        assert_eq!(window.len(), 7);

        let rule = BurstRule {
            id: "loadout_restore".into(),
            anchor: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            member: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            tags: vec!["Inventory".into()],
            min_burst_size: 3,
            max_member_gap: 0,
        };
        let hits = detect_bursts(&window, std::slice::from_ref(&rule));
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.rule_id, "loadout_restore");
        assert_eq!(hit.start_index, 0);
        assert_eq!(hit.end_index, 5);
        assert_eq!(hit.size, 6);
        assert_eq!(hit.member_indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn burst_time_gap_ceiling_splits_cross_session_runs() {
        // Two 3-line AttachmentReceived runs, contiguous in the
        // offset-sorted window (as they appear after a Game.log rotation
        // resets source_offset) but ~2.5h apart in wall-clock time. The
        // unconstrained live path welds them into one 6-member burst; the
        // reparse ceiling must split them so no burst — and no member
        // deletion — spans the session boundary.
        const CROSS_SESSION: &str = "\
<2026-05-02T21:15:03.053Z> <AttachmentReceived> a1 [Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> a2 [Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> a3 [Inventory]
<2026-05-02T23:45:00.000Z> <AttachmentReceived> b1 [Inventory]
<2026-05-02T23:45:00.000Z> <AttachmentReceived> b2 [Inventory]
<2026-05-02T23:45:00.000Z> <AttachmentReceived> b3 [Inventory]
";
        let window = lines(CROSS_SESSION);
        assert_eq!(window.len(), 6);
        let rule = BurstRule {
            id: "loadout_restore".into(),
            anchor: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            member: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            tags: vec!["Inventory".into()],
            min_burst_size: 3,
            max_member_gap: 0,
        };

        // Live path (no ceiling) welds the two sessions — the bug.
        let welded = detect_bursts(&window, std::slice::from_ref(&rule));
        assert_eq!(welded.len(), 1);
        assert_eq!(welded[0].size, 6);

        // Reparse path (120s ceiling) splits into two disjoint bursts;
        // neither spans the boundary.
        let split = detect_bursts_with_time_gap(&window, std::slice::from_ref(&rule), 120);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].member_indices, vec![0, 1, 2]);
        assert_eq!(split[1].member_indices, vec![3, 4, 5]);
    }

    #[test]
    fn burst_time_gap_ceiling_keeps_tight_run_intact() {
        // A single legitimate run (all members within the ceiling) stays
        // one burst under the constrained path — the ceiling only splits
        // genuine wall-clock gaps, not within-session spam.
        let window = lines(ATTACHMENT_BURST_FIXTURE);
        let rule = BurstRule {
            id: "loadout_restore".into(),
            anchor: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            member: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            tags: vec!["Inventory".into()],
            min_burst_size: 3,
            max_member_gap: 0,
        };
        let hits = detect_bursts_with_time_gap(&window, std::slice::from_ref(&rule), 120);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].size, 6);
    }

    #[test]
    fn burst_min_size_below_threshold_no_hit() {
        // Two AttachmentReceived in a row — short of min=3.
        const SHORT: &str = "\
<T1> <AttachmentReceived> body [Inventory]
<T2> <AttachmentReceived> body [Inventory]
<T3> <SomethingElse> body
";
        let window = lines(SHORT);
        let rule = BurstRule {
            id: "loadout_restore".into(),
            anchor: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            member: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            tags: vec!["Inventory".into()],
            min_burst_size: 3,
            max_member_gap: 0,
        };
        assert!(detect_bursts(&window, std::slice::from_ref(&rule)).is_empty());
    }

    #[test]
    fn burst_gap_budget_keeps_burst_alive_across_one_unrelated_line() {
        // Burst with one InventoryManagement line in the middle.
        // max_member_gap=1 keeps the burst alive across that gap.
        const WITH_GAP: &str = "\
<T1> <AttachmentReceived> body Player[X] [Team_X][Inventory]
<T2> <AttachmentReceived> body Player[X] [Team_X][Inventory]
<T3> <InventoryManagement> body Player[X] [Team_X][Inventory]
<T4> <AttachmentReceived> body Player[X] [Team_X][Inventory]
<T5> <AttachmentReceived> body Player[X] [Team_X][Inventory]
<T6> <Unrelated> body
";
        let window = lines(WITH_GAP);
        let rule = BurstRule {
            id: "loadout_restore".into(),
            anchor: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            member: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            tags: vec!["Inventory".into()],
            min_burst_size: 4,
            max_member_gap: 1,
        };
        let hits = detect_bursts(&window, std::slice::from_ref(&rule));
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.size, 4);
        // The InventoryManagement line at index 2 is NOT a member —
        // it's the tolerated gap.
        assert_eq!(hit.member_indices, vec![0, 1, 3, 4]);
    }

    #[test]
    fn burst_terrain_load_spam_collapses() {
        // Same shape as the loadout burst but for `<StatObjLoad>`
        // events during planet entry. Confirms the rule generalises
        // across event_names.
        const TERRAIN: &str = "\
<T1> <StatObjLoad 0x800 Format> 'data/.../node_1.cgf' - File exists in P4K [Team_Graphics][CoreTech]
<T2> <StatObjLoad 0x800 Format> 'data/.../node_2.cgf' - File exists in P4K [Team_Graphics][CoreTech]
<T3> <StatObjLoad 0x800 Format> 'data/.../node_3.cgf' - File exists in P4K [Team_Graphics][CoreTech]
<T4> <StatObjLoad 0x800 Format> 'data/.../node_4.cgf' - File exists in P4K [Team_Graphics][CoreTech]
<T5> <Unrelated> body
";
        let window = lines(TERRAIN);
        let rule = BurstRule {
            id: "terrain_load_burst".into(),
            anchor: StepMatch::EventName {
                name: "StatObjLoad 0x800 Format".into(),
            },
            member: StepMatch::EventName {
                name: "StatObjLoad 0x800 Format".into(),
            },
            tags: vec!["CoreTech".into()],
            min_burst_size: 3,
            max_member_gap: 0,
        };
        let hits = detect_bursts(&window, std::slice::from_ref(&rule));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].size, 4);
    }

    #[test]
    fn burst_two_disjoint_bursts_in_same_window() {
        const TWO: &str = "\
<T1> <AttachmentReceived> body [Inventory]
<T2> <AttachmentReceived> body [Inventory]
<T3> <AttachmentReceived> body [Inventory]
<T4> <Unrelated> body
<T5> <Unrelated> body
<T6> <AttachmentReceived> body [Inventory]
<T7> <AttachmentReceived> body [Inventory]
<T8> <AttachmentReceived> body [Inventory]
";
        let window = lines(TWO);
        let rule = BurstRule {
            id: "loadout_restore".into(),
            anchor: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            member: StepMatch::EventName {
                name: "AttachmentReceived".into(),
            },
            tags: vec!["Inventory".into()],
            min_burst_size: 3,
            max_member_gap: 0,
        };
        let hits = detect_bursts(&window, std::slice::from_ref(&rule));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].member_indices, vec![0, 1, 2]);
        assert_eq!(hits[1].member_indices, vec![5, 6, 7]);
    }

    #[test]
    fn burst_multiple_rules_first_rule_wins() {
        // Two rules both want to match AttachmentReceived. First one
        // declared consumes the burst; second one finds no anchors.
        const SHARED: &str = "\
<T1> <AttachmentReceived> body [Inventory]
<T2> <AttachmentReceived> body [Inventory]
<T3> <AttachmentReceived> body [Inventory]
";
        let window = lines(SHARED);
        let rules = vec![
            BurstRule {
                id: "first".into(),
                anchor: StepMatch::EventName {
                    name: "AttachmentReceived".into(),
                },
                member: StepMatch::EventName {
                    name: "AttachmentReceived".into(),
                },
                tags: vec!["Inventory".into()],
                min_burst_size: 3,
                max_member_gap: 0,
            },
            BurstRule {
                id: "second".into(),
                anchor: StepMatch::EventName {
                    name: "AttachmentReceived".into(),
                },
                member: StepMatch::EventName {
                    name: "AttachmentReceived".into(),
                },
                tags: vec!["Inventory".into()],
                min_burst_size: 3,
                max_member_gap: 0,
            },
        ];
        let hits = detect_bursts(&window, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule_id, "first");
    }

    #[test]
    fn burst_empty_window_returns_no_hits() {
        let rule = BurstRule {
            id: "x".into(),
            anchor: StepMatch::EventName { name: "Y".into() },
            member: StepMatch::EventName { name: "Y".into() },
            tags: vec![],
            min_burst_size: 1,
            max_member_gap: 0,
        };
        assert!(detect_bursts(&[], std::slice::from_ref(&rule)).is_empty());
    }
}

#[cfg(test)]
mod loadout_categorise_tests {
    use super::{build_loadout_categories, categorise_loadout_item};

    #[test]
    fn known_weapon_prefix() {
        assert_eq!(categorise_loadout_item("WEAPON_lr2_p4ar"), "weapons");
    }

    #[test]
    fn weapon_substring_pattern() {
        assert_eq!(categorise_loadout_item("xyz_rifle_a01"), "weapons");
    }

    #[test]
    fn armor_prefix() {
        assert_eq!(categorise_loadout_item("ARMOR_pilotsuit_v3"), "armor");
    }

    #[test]
    fn attachment_prefix() {
        assert_eq!(categorise_loadout_item("ATT_scope_4x"), "attachments");
    }

    #[test]
    fn consumable_prefix() {
        assert_eq!(
            categorise_loadout_item("CONSUM_medpen_basic"),
            "consumables"
        );
    }

    #[test]
    fn unknown_falls_back() {
        assert_eq!(categorise_loadout_item("MISC_random_thing"), "unknown");
    }

    // -----------------------------------------------------------------
    // build_loadout_categories — payload builder helper
    // -----------------------------------------------------------------

    /// Synthetic loadout-restore burst with a known mix of item ids.
    /// Mirrors the real fixture in ATTACHMENT_BURST_FIXTURE but uses
    /// explicit item_class strings (the classify pass already extracted
    /// them from the raw lines before this helper is called).
    const BURST_ITEM_CLASSES: &[&str] = &[
        "body_01_noMagicPocket", // → unknown (body anchor, not a gear item)
        "vgl_undersuit_01",      // → armor (_undersuit_ substring)
        "rrs_arms_03",           // → armor (_arms_ substring)
        "rrs_helmet_03",         // → armor (_helmet_ substring)
        "rrs_legs_03",           // → armor (_legs_ substring)
        "klwe_pistol_01",        // → weapons (_pistol_ substring)
    ];

    #[test]
    fn build_loadout_categories_counts_by_bucket() {
        let cats = build_loadout_categories(BURST_ITEM_CLASSES)
            .expect("non-empty input should produce Some(map)");

        // vgl_undersuit_01 is now correctly classified as armor via _undersuit_
        assert_eq!(
            cats["armor"], 4,
            "four armor pieces expected (incl. undersuit)"
        );
        assert_eq!(cats["weapons"], 1, "one pistol expected");
        assert_eq!(
            cats["unknown"], 1,
            "one uncategorised item expected (body anchor)"
        );
        // Buckets with zero items must be absent (not emitted as 0).
        assert!(
            !cats.contains_key("attachments"),
            "no attachments in fixture"
        );
        assert!(
            !cats.contains_key("consumables"),
            "no consumables in fixture"
        );
    }

    #[test]
    fn build_loadout_categories_empty_input_returns_none() {
        let empty: &[&str] = &[];
        assert!(build_loadout_categories(empty).is_none());
    }

    #[test]
    fn build_loadout_categories_pure_weapons_burst() {
        let items = ["WEAPON_lr2_p4ar", "wpn_ballistic_ar_01", "xyz_rifle_mk2"];
        let cats = build_loadout_categories(&items).unwrap();
        assert_eq!(cats["weapons"], 3);
        assert_eq!(cats.len(), 1, "only the weapons bucket should be present");
    }

    // -----------------------------------------------------------------
    // New weapon patterns
    // -----------------------------------------------------------------

    #[test]
    fn weapon_ar_substring_pattern() {
        assert_eq!(categorise_loadout_item("klwe_ar_s1_01"), "weapons");
    }

    #[test]
    fn weapon_lmg_substring_pattern() {
        assert_eq!(categorise_loadout_item("behr_lmg_01_white"), "weapons");
    }

    #[test]
    fn weapon_smg_substring_pattern() {
        assert_eq!(categorise_loadout_item("behr_smg_02_stealth"), "weapons");
    }

    #[test]
    fn weapon_dmr_substring_pattern() {
        assert_eq!(categorise_loadout_item("apar_dmr_01_green"), "weapons");
    }

    #[test]
    fn weapon_sniper_substring_pattern() {
        assert_eq!(categorise_loadout_item("behr_sniper_01_scope"), "weapons");
    }

    #[test]
    fn weapon_shotgun_substring_pattern() {
        assert_eq!(categorise_loadout_item("klwe_shotgun_01_grip"), "weapons");
    }

    #[test]
    fn weapon_ballistic_substring_pattern() {
        assert_eq!(
            categorise_loadout_item("behr_ballistic_pistol_01"),
            "weapons"
        );
    }

    #[test]
    fn weapon_energy_substring_pattern() {
        assert_eq!(categorise_loadout_item("volt_energy_rifle_02"), "weapons");
    }

    #[test]
    fn weapon_ksar_manufacturer_prefix() {
        assert_eq!(categorise_loadout_item("KSAR_ar_01_iron"), "weapons");
    }

    #[test]
    fn weapon_gmni_manufacturer_prefix() {
        assert_eq!(categorise_loadout_item("GMNI_lmg_v2_black"), "weapons");
    }

    // -----------------------------------------------------------------
    // New armor patterns
    // -----------------------------------------------------------------

    #[test]
    fn armor_undersuit_substring_pattern() {
        assert_eq!(categorise_loadout_item("vgl_undersuit_01_rsi"), "armor");
    }

    #[test]
    fn armor_backpack_substring_pattern() {
        assert_eq!(categorise_loadout_item("rrs_backpack_03_gray"), "armor");
    }

    #[test]
    fn armor_chest_substring_pattern() {
        assert_eq!(categorise_loadout_item("rrs_chest_04_heavy"), "armor");
    }

    #[test]
    fn armor_helmet_prefix() {
        assert_eq!(categorise_loadout_item("helmet_rrs_01_green"), "armor");
    }

    #[test]
    fn armor_heavy_armor_substring() {
        assert_eq!(categorise_loadout_item("rsi_heavy_armor_v2"), "armor");
    }

    #[test]
    fn armor_medium_armor_substring() {
        assert_eq!(categorise_loadout_item("cru_medium_armor_01"), "armor");
    }

    #[test]
    fn armor_light_armor_substring() {
        assert_eq!(categorise_loadout_item("argo_light_armor_std"), "armor");
    }

    // -----------------------------------------------------------------
    // New attachment patterns
    // -----------------------------------------------------------------

    #[test]
    fn attachment_sight_substring_pattern() {
        assert_eq!(categorise_loadout_item("klwe_sight_holo_02"), "attachments");
    }

    #[test]
    fn attachment_barrel_substring_pattern() {
        assert_eq!(categorise_loadout_item("apx_barrel_ext_01"), "attachments");
    }

    #[test]
    fn attachment_muzzle_substring_pattern() {
        assert_eq!(
            categorise_loadout_item("behr_muzzle_flash_01"),
            "attachments"
        );
    }

    #[test]
    fn attachment_suppressor_substring_pattern() {
        assert_eq!(
            categorise_loadout_item("klwe_suppressor_01_std"),
            "attachments"
        );
    }

    #[test]
    fn attachment_compensator_substring_pattern() {
        assert_eq!(
            categorise_loadout_item("behr_compensator_02"),
            "attachments"
        );
    }

    #[test]
    fn attachment_underbarrel_substring_pattern() {
        assert_eq!(
            categorise_loadout_item("apx_underbarrel_laser_01"),
            "attachments"
        );
    }

    // -----------------------------------------------------------------
    // New consumable patterns
    // -----------------------------------------------------------------

    #[test]
    fn consumable_hempen_substring_pattern() {
        assert_eq!(categorise_loadout_item("uee_hempen_01_std"), "consumables");
    }

    #[test]
    fn consumable_adrenpen_substring_pattern() {
        assert_eq!(categorise_loadout_item("uee_adrenpen_01"), "consumables");
    }

    #[test]
    fn consumable_detrapen_substring_pattern() {
        assert_eq!(categorise_loadout_item("uee_detrapen_01"), "consumables");
    }

    #[test]
    fn consumable_oxypen_substring_pattern() {
        assert_eq!(categorise_loadout_item("uee_oxypen_01"), "consumables");
    }

    #[test]
    fn consumable_frag_grenade_substring_pattern() {
        assert_eq!(
            categorise_loadout_item("behring_frag_01_std"),
            "consumables"
        );
    }

    #[test]
    fn consumable_flashbang_substring_pattern() {
        assert_eq!(categorise_loadout_item("uee_flashbang_01"), "consumables");
    }

    #[test]
    fn consumable_smoke_grenade_substring_pattern() {
        assert_eq!(
            categorise_loadout_item("uee_smoke_grenade_01"),
            "consumables"
        );
    }

    #[test]
    fn consumable_emp_grenade_substring_pattern() {
        assert_eq!(categorise_loadout_item("uee_emp_grenade_01"), "consumables");
    }
}

#[cfg(test)]
mod loadout_items_tests {
    use super::build_loadout_items;

    #[test]
    fn build_loadout_items_sets_category_and_preserves_port() {
        let pairs = vec![
            ("rsi_p4ar_01".to_string(), "WEAPON_LEFT".to_string()),
            (
                "rsi_odyssey_helmet_01_01_01".to_string(),
                "Armor_Helmet".to_string(),
            ),
        ];
        let items = build_loadout_items(&pairs).expect("non-empty");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].class, "rsi_p4ar_01");
        assert_eq!(items[0].port, "WEAPON_LEFT");
        assert_eq!(items[0].category, "weapons");
        assert_eq!(items[1].category, "armor");
        assert!(build_loadout_items(&[]).is_none());
    }
}
