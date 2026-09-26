//! Review-queue triage: which unknown-line groups are worth a person's
//! attention, and which are engine chatter to tuck away.
//!
//! The queue used to list every captured SHAPE. Measured on one real tray
//! database on 2026-09-26: 342,428 open shapes, 99.7% of them scoring
//! 50–59 against a review threshold of 50, so the threshold filtered almost
//! nothing; 47% were seen exactly once. Grouped by the log line's shell tag
//! the same data is 398 groups, and the top four inventory tags alone are
//! ~240k shapes.
//!
//! Volume is the wrong signal for "featured": ranked by occurrences the top
//! of that list was inventory, routing, voice-chat and loading internals.
//! What makes a line worth reviewing is that it reads like something that
//! HAPPENED TO THE PLAYER — a mission ending, a quantum arrival, a contract
//! being generated — because that is what a parser rule can turn into a
//! metric. So a group is featured when its tag uses gameplay vocabulary and
//! is not a known plumbing family; everything else goes to Other, which the
//! pane collapses. On the same database that split was 118 featured
//! candidates to 280 Other, and the featured list is then capped.

use serde::{Deserialize, Serialize};

/// Featured groups shown at once. A person reviews these by hand; a longer
/// list is the "too many review items" problem again.
pub const FEATURED_CAP: usize = 25;

/// Aggregate of every open shape sharing one shell tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewGroupStats {
    /// The log line's `<Tag>`.
    pub shell_tag: String,
    /// Distinct shapes (variants) in the group.
    pub shapes: u64,
    /// Total occurrences across those shapes.
    pub occurrences: u64,
    pub last_seen: String,
    pub max_interest: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Featured,
    Other,
}

/// Words that mark a tag as describing something the player did or had
/// happen to them. Matched case-insensitively as substrings, so `Mission`
/// covers `MissionEnded` and `CLocalMissionPhaseMarker`.
const GAMEPLAY: &[&str] = &[
    "kill",
    "death",
    "died",
    "incap",
    "injur",
    "medic",
    "respawn",
    "mission",
    "contract",
    "objective",
    "bounty",
    "cargo",
    "haul",
    "mining",
    "salvage",
    "refin",
    "quantum",
    "jump",
    "landing",
    "takeoff",
    "dock",
    "hangar",
    "spawn",
    "purchase",
    "shop",
    "reward",
    "payout",
    "fine",
    "crime",
    "arrest",
    "prison",
    "insurance",
    "claim",
    "party",
    "vehicle",
    "destroy",
    "damage",
    "weapon",
    "shield",
    "repair",
    "refuel",
    "rearm",
    "trade",
    "commodity",
    "loot",
    "rescue",
    "beacon",
    "elevator",
    "tram",
    "train",
];

/// Plumbing families that match gameplay words by accident (an
/// inventory-grid "request", a route "calculate") or are known chatter.
/// Checked as case-insensitive prefixes of the tag, or substrings where
/// marked by a leading `*`.
const PLUMBING: &[&str] = &[
    "*inventory",
    "*query",
    "request",
    "channel",
    "connection",
    "webrtc",
    "eos_",
    "actor physics",
    "context",
    "cscloading",
    "loading",
    "stream",
    "found obstruction",
    "calculate route",
    "failed to get starmap",
    "generatelocationproperty",
    "local route guard",
    "subscribeto",
    "*uiprovider",
];

fn is_plumbing(tag_lc: &str) -> bool {
    PLUMBING.iter().any(|p| match p.strip_prefix('*') {
        Some(sub) => tag_lc.contains(sub),
        None => tag_lc.starts_with(p),
    })
}

pub fn tier(shell_tag: &str) -> Tier {
    let lc = shell_tag.to_ascii_lowercase();
    if lc.is_empty() || is_plumbing(&lc) {
        return Tier::Other;
    }
    if GAMEPLAY.iter().any(|w| lc.contains(w)) {
        Tier::Featured
    } else {
        Tier::Other
    }
}

/// Ordering within the featured list. Frequency counts (log-scaled, so one
/// firehose does not bury everything), but a tag written as a readable
/// message ("Quantum Drive Arrived - …") ranks ahead of an engine symbol
/// (`CFoo::Bar`) at similar volume: the message is what the game chose to
/// narrate, the symbol is usually its implementation detail. Groups not
/// seen in the fortnight before the newest capture sink, so a line from a
/// retired game build does not hold a slot.
pub fn rank(g: &ReviewGroupStats, newest_last_seen: &str) -> f64 {
    let mut score = ((g.occurrences as f64) + 1.0).log10() * 10.0;
    let readable = g.shell_tag.contains(' ') && !g.shell_tag.contains("::");
    if readable {
        score += 8.0;
    }
    if days_between(&g.last_seen, newest_last_seen).unwrap_or(0) > 14 {
        score -= 15.0;
    }
    score
}

fn days_between(earlier: &str, later: &str) -> Option<i64> {
    let a = chrono::DateTime::parse_from_rfc3339(earlier).ok()?;
    let b = chrono::DateTime::parse_from_rfc3339(later).ok()?;
    Some((b - a).num_days())
}

/// Split groups into featured (ranked, capped) and Other (by volume, so the
/// biggest chatter is first in line for "ignore all").
pub fn triage(groups: Vec<ReviewGroupStats>) -> (Vec<ReviewGroupStats>, Vec<ReviewGroupStats>) {
    let newest = groups
        .iter()
        .map(|g| g.last_seen.as_str())
        .max()
        .unwrap_or("")
        .to_string();
    let (mut featured, mut other): (Vec<_>, Vec<_>) = groups
        .into_iter()
        .partition(|g| tier(&g.shell_tag) == Tier::Featured);
    featured.sort_by(|a, b| {
        rank(b, &newest)
            .partial_cmp(&rank(a, &newest))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.shell_tag.cmp(&b.shell_tag))
    });
    if featured.len() > FEATURED_CAP {
        other.extend(featured.split_off(FEATURED_CAP));
    }
    other.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.shell_tag.cmp(&b.shell_tag))
    });
    (featured, other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(tag: &str, occ: u64, last_seen: &str) -> ReviewGroupStats {
        ReviewGroupStats {
            shell_tag: tag.to_string(),
            shapes: 1,
            occurrences: occ,
            last_seen: last_seen.to_string(),
            max_interest: 55,
        }
    }

    const NOW: &str = "2026-09-19T22:00:00Z";

    #[test]
    fn gameplay_tags_from_the_real_corpus_are_featured() {
        for tag in [
            "MissionEnded",
            "ObjectiveUpserted",
            "Quantum Drive Arrived - Arrived at Final Destination",
            "Cannot Align - No Jump Point",
            "CContractGenerator::CContractGenerator",
            "Bad Spawn Position",
        ] {
            assert_eq!(tier(tag), Tier::Featured, "{tag}");
        }
    }

    #[test]
    fn the_measured_chatter_families_go_to_other() {
        // The top of the list when ranked by volume, 2026-09-26.
        for tag in [
            "InventoryManagement",
            "Inventory Token Flow",
            "RequestInventory",
            "Query Inventory",
            "Failed to get starmap route data!",
            "Calculate Route",
            "GenerateLocationProperty",
            "WebRTC/Janus",
            "EOS_Logging",
            "Channel Connection Complete",
            "VehicleListQuery",
            "SubscribeToFriendMessages",
            "CEntityComponentCommodityUIProvider::AddPlayerCommodityItem",
            "",
        ] {
            assert_eq!(tier(tag), Tier::Other, "{tag}");
        }
    }

    #[test]
    fn readable_messages_outrank_engine_symbols_at_similar_volume() {
        let msg = g("Quantum Drive Arrived - Final", 1000, NOW);
        let sym = g("CObjectiveMarkerComponent::RWES", 1500, NOW);
        assert!(rank(&msg, NOW) > rank(&sym, NOW));
    }

    #[test]
    fn stale_groups_sink() {
        let fresh = g("MissionEnded", 100, NOW);
        let stale = g("MissionStarted", 100, "2026-08-01T00:00:00Z");
        assert!(rank(&fresh, NOW) > rank(&stale, NOW));
    }

    #[test]
    fn featured_is_capped_and_the_overflow_joins_other() {
        let mut groups: Vec<ReviewGroupStats> = (0..40)
            .map(|i| g(&format!("Mission Step {i}"), 10 + i, NOW))
            .collect();
        groups.push(g("InventoryManagement", 1_000_000, NOW));
        let (featured, other) = triage(groups);
        assert_eq!(featured.len(), FEATURED_CAP);
        assert_eq!(other.len(), 41 - FEATURED_CAP);
        assert_eq!(
            other[0].shell_tag, "InventoryManagement",
            "Other is by volume"
        );
        assert!(featured[0].occurrences >= featured[FEATURED_CAP - 1].occurrences);
    }
}
