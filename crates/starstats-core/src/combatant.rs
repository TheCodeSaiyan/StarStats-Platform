//! Engine entity name → something a reader can recognise.
//!
//! The `<Actor Death>` line names its combatants the way the engine
//! stores them, not the way a person would say them:
//!
//! ```text
//! PU_Pilots-Human-Criminal-Pilot_Light_9823746
//! Kopion_Irradiated_7712094
//! ungolian
//! ```
//!
//! The first two are NPCs, the third is a player handle. Shown raw, a
//! "top enemies" list is a column of engine identifiers; humanised, it
//! is "Criminal Pilot Light" and "Kopion Irradiated".
//!
//! # Why this lives in core
//!
//! Same reason as [`crate::location_classifier`]: the tray and the
//! server both need the same answer, and the crate is I/O-free so both
//! can hold it. It is NOT stamped onto stored events — per the
//! architecture rule that classification is derived at query time, so
//! that improving the rules improves every row already in the database
//! rather than only those ingested afterwards.
//!
//! # What it is honest about
//!
//! Every name the rules cannot place lands in
//! [`CombatantFamily::Unclassified`] with the raw name preserved, and
//! callers are expected to SHOW that rather than drop it. A "top
//! enemies" list that silently omits what it failed to parse is a list
//! that lies about its own total.

use serde::{Deserialize, Serialize};

/// What kind of thing a combatant name refers to.
///
/// Deliberately coarse. The engine's naming carries far more detail
/// than this (faction, tier, loadout), but the shapes are known only
/// from the handful observed in production data, and a taxonomy
/// invented ahead of the evidence would be confidently wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatantFamily {
    /// Human NPC — `PU_Pilots-*`, `NPC_Archetypes-*`, `PU_Human-*`.
    Human,
    /// Fauna — `Kopion_*`, `vlk_*` (Valakkar), and friends.
    Creature,
    /// Not an actor at all: the engine attributes an environmental
    /// death to a stand-in name (a fall, suffocation, a crash). These
    /// are the rows that must never be counted as kills.
    Environment,
    /// A bare name with no engine scaffolding — a player-handle shape.
    ///
    /// A kill of one of these is no longer something the log reports,
    /// so in practice this is the SUBJECT of a death rather than the
    /// victim of a kill.
    PlayerLike,
    /// No rule matched. Counted and shown, never hidden.
    Unclassified,
}

/// A combatant name, humanised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatantLabel {
    /// What to show a reader.
    pub display: String,
    pub family: CombatantFamily,
    /// The name with its trailing entity id removed, which is what to
    /// group by: `Kopion_Irradiated_7712094` and
    /// `Kopion_Irradiated_9910233` are two encounters with one enemy,
    /// and grouping on the raw name makes every encounter unique —
    /// producing a "top enemies" list where every entry is 1.
    pub group_key: String,
}

/// Engine scaffolding that carries no meaning for a reader.
///
/// `PU` is the persistent-universe prefix, `NPC` / `Archetypes` say
/// what the record IS rather than what it is called, and `Pilots` is
/// restated by the `Pilot` token that follows it.
const NOISE_TOKENS: &[&str] = &["pu", "npc", "archetypes", "pilots"];

/// Names the engine puts in the killer slot for a death nobody dealt.
///
/// `JackAndJillFell` is the fall: 572 of one production handle's
/// `actor_death` rows carry it. Counting those as kills is what the
/// self-kill fix in `combat_counts` removed at the SQL layer; this is
/// the same fact, made available to anything holding a name.
const ENVIRONMENT_NAMES: &[&str] = &[
    "jackandjillfell",
    "unknown",
    "suffocation",
    "crash",
    "hazard",
];

/// Prefixes that name fauna rather than people.
const CREATURE_PREFIXES: &[&str] = &["kopion", "vlk", "valakkar", "marok"];

/// Strip a trailing `_<digits>` entity id.
///
/// The engine appends a globally-unique id to every spawned entity, so
/// one enemy archetype appears under thousands of distinct names.
pub fn strip_entity_id(raw: &str) -> &str {
    match raw.rsplit_once('_') {
        Some((head, tail))
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) && !head.is_empty() =>
        {
            head
        }
        _ => raw,
    }
}

/// Humanise one combatant name.
pub fn humanize_combatant(raw: &str) -> CombatantLabel {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return CombatantLabel {
            display: "Unknown".to_string(),
            family: CombatantFamily::Unclassified,
            group_key: String::new(),
        };
    }

    let group_key = strip_entity_id(trimmed).to_string();
    let lower = group_key.to_ascii_lowercase();

    if ENVIRONMENT_NAMES.contains(&lower.as_str()) {
        return CombatantLabel {
            display: environment_display(&lower),
            family: CombatantFamily::Environment,
            group_key,
        };
    }

    let is_creature = CREATURE_PREFIXES.iter().any(|p| lower.starts_with(p));

    // The engine mixes `-` and `_` as separators within one name
    // (`PU_Pilots-Human-Criminal-Pilot_Light`), so both split.
    let tokens: Vec<&str> = group_key
        .split(['-', '_'])
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.len() == 1 && !is_creature {
        // No scaffolding at all. That is what a player handle looks like,
        // and a handle is spelled the way its owner spells it.
        return CombatantLabel {
            display: group_key.clone(),
            family: CombatantFamily::PlayerLike,
            group_key,
        };
    }

    let family = if is_creature {
        CombatantFamily::Creature
    } else if lower.starts_with("pu_") || lower.starts_with("npc_") || lower.contains("pilots") {
        CombatantFamily::Human
    } else {
        CombatantFamily::Unclassified
    };

    let meaningful: Vec<String> = tokens
        .iter()
        .filter(|t| !NOISE_TOKENS.contains(&t.to_ascii_lowercase().as_str()))
        .map(|t| title_case(t))
        .collect();

    let display = if meaningful.is_empty() {
        // Every token was scaffolding. Better the raw name than nothing.
        group_key.clone()
    } else {
        meaningful.join(" ")
    };

    CombatantLabel {
        display,
        family,
        group_key,
    }
}

fn environment_display(lower: &str) -> String {
    match lower {
        "jackandjillfell" => "A fall",
        "suffocation" => "Suffocation",
        "crash" => "A crash",
        "hazard" => "An environmental hazard",
        _ => "Unknown cause",
    }
    .to_string()
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut next_upper = true;
    for ch in s.chars() {
        if next_upper {
            out.extend(ch.to_uppercase());
            next_upper = false;
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes below are what production actually holds. They came off
    /// a `GROUP BY` over one handle's `actor_death` rows:
    ///
    /// ```text
    /// PU_Pilots-Human-Criminal-Pilot_Light   1074
    /// Kopion_Irradiated                      1046
    /// JackAndJillFell                         572
    /// ```
    #[test]
    fn humanizes_the_victim_shapes_production_holds() {
        let pilot = humanize_combatant("PU_Pilots-Human-Criminal-Pilot_Light_9823746");
        assert_eq!(pilot.display, "Human Criminal Pilot Light");
        assert_eq!(pilot.family, CombatantFamily::Human);
        assert_eq!(pilot.group_key, "PU_Pilots-Human-Criminal-Pilot_Light");

        let kopion = humanize_combatant("Kopion_Irradiated_7712094");
        assert_eq!(kopion.display, "Kopion Irradiated");
        assert_eq!(kopion.family, CombatantFamily::Creature);

        let fall = humanize_combatant("JackAndJillFell");
        assert_eq!(fall.family, CombatantFamily::Environment);
        assert_eq!(fall.display, "A fall");
    }

    /// The entity id is why grouping on the raw name is useless: every
    /// spawned enemy carries a different one, so a "top enemies" list
    /// built on raw names has a thousand rows all equal to 1.
    #[test]
    fn two_spawns_of_one_archetype_share_a_group_key() {
        let a = humanize_combatant("Kopion_Irradiated_7712094");
        let b = humanize_combatant("Kopion_Irradiated_9910233");
        assert_eq!(a.group_key, b.group_key);
        assert_eq!(a.display, b.display);
    }

    #[test]
    fn a_bare_name_reads_as_a_player_handle_and_is_left_alone() {
        let p = humanize_combatant("ungolian");
        assert_eq!(p.family, CombatantFamily::PlayerLike);
        // NOT title-cased: a handle is spelled how its owner spells it.
        assert_eq!(p.display, "ungolian");
    }

    /// A trailing number that is part of the NAME must not be eaten.
    #[test]
    fn strip_entity_id_leaves_a_name_that_is_only_digits_alone() {
        assert_eq!(strip_entity_id("12345"), "12345");
        assert_eq!(strip_entity_id("Kopion_Irradiated"), "Kopion_Irradiated");
        assert_eq!(strip_entity_id("Kopion_Irradiated_77"), "Kopion_Irradiated");
    }

    /// The point of the variant: an unrecognised shape is still counted.
    #[test]
    fn an_unknown_shape_is_unclassified_rather_than_dropped() {
        let u = humanize_combatant("Xeno-Thing_Weird_42");
        assert_eq!(u.family, CombatantFamily::Unclassified);
        assert_eq!(u.display, "Xeno Thing Weird");
        assert!(!u.group_key.is_empty());
    }

    #[test]
    fn an_empty_name_does_not_panic() {
        let e = humanize_combatant("   ");
        assert_eq!(e.family, CombatantFamily::Unclassified);
        assert_eq!(e.display, "Unknown");
    }
}
