//! Contract taxonomy — the closed vocabulary a contract is classified into,
//! plus the normalisers for the two other free-text fields the extractor emits
//! inconsistently (`risk`, `legal_status`).
//!
//! Contracts arrive from sp-ingest as LLM-extracted free text. Measured across
//! the live catalogue on 2026-08-06: `contract_type` had 16 distinct values
//! including `Salvage` AND `SALVAGE`, and `Cargo Haul` / `Cargo Recovery` /
//! `Cargo Retrieval` for one concept; `risk` was case-split (`medium`=37 vs
//! `Medium`=24); `legal_status` was `Legal`/`Lawful`/NULL. Every consumer that
//! wants to group or filter contracts had to re-solve that.
//!
//! **Derived at query time, never denormalised** — the same architecture the
//! location taxonomy v2 landed on (see `location_taxonomy`), and for the same
//! reason: it eliminates the backfill entirely, so no migration is needed and
//! the additive-only / byte-immutable migration rule is never engaged.
//!
//! The raw values are still published unchanged. This module adds a derived
//! view alongside them; it does not replace or rewrite what was ingested.
//!
//! Classification reads `contract_type` and `gameplay_loop` ONLY. Both are
//! promoted columns on `contracts`, so the list and detail paths necessarily
//! agree. Deliberately NOT step types: `ContractSummaryRow` is promoted-columns
//! only (no JSONB read), so a step-aware classifier would either cost a JSONB
//! read per list row or let list and detail disagree about the same contract.

use serde::{Deserialize, Serialize};

/// Parent category a contract belongs to.
///
/// Closed vocabulary, emitted as snake_case TEXT. Per the house rule for
/// closed-vocabulary enums, adding a variant needs no migration — nothing
/// stores this, it is computed on read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractCategory {
    /// Tracking and apprehending/eliminating a named target for a bounty.
    BountyHunter,
    /// Paid combat that is not a named-target bounty: extermination, area
    /// clearance, arena work.
    Mercenary,
    /// Moving cargo, including recovery and retrieval of cargo already lost.
    Hauling,
    /// Point-to-point delivery of an item or package. Distinct from `Hauling`
    /// by scale, not by activity.
    Delivery,
    /// Stripping wrecks and derelicts.
    Salvage,
    /// Extracting raw resources.
    ///
    /// No contract in the live catalogue declares this as its `contract_type`
    /// today; it is reachable via `gameplay_loop` and retained because
    /// StarPlatform's operational vocabulary already has a mining category.
    Mining,
    /// Fetching a specific item or set of items that are not cargo.
    Collection,
    /// Going somewhere to find something out.
    Investigation,
    /// Servicing a ship or installation: refuelling, repair.
    Maintenance,
    /// Instructional or certification work.
    Training,
    /// Recognised but uncategorised. A real destination, not an error case —
    /// an unrecognised type is still published, never dropped.
    Other,
}

impl ContractCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BountyHunter => "bounty_hunter",
            Self::Mercenary => "mercenary",
            Self::Hauling => "hauling",
            Self::Delivery => "delivery",
            Self::Salvage => "salvage",
            Self::Mining => "mining",
            Self::Collection => "collection",
            Self::Investigation => "investigation",
            Self::Maintenance => "maintenance",
            Self::Training => "training",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "bounty_hunter" => Self::BountyHunter,
            "mercenary" => Self::Mercenary,
            "hauling" => Self::Hauling,
            "delivery" => Self::Delivery,
            "salvage" => Self::Salvage,
            "mining" => Self::Mining,
            "collection" => Self::Collection,
            "investigation" => Self::Investigation,
            "maintenance" => Self::Maintenance,
            "training" => Self::Training,
            "other" => Self::Other,
            _ => return None,
        })
    }

    /// Every variant, for exhaustive tests and for enumerating the filter's
    /// accepted values in API docs.
    pub const ALL: [Self; 11] = [
        Self::BountyHunter,
        Self::Mercenary,
        Self::Hauling,
        Self::Delivery,
        Self::Salvage,
        Self::Mining,
        Self::Collection,
        Self::Investigation,
        Self::Maintenance,
        Self::Training,
        Self::Other,
    ];
}

/// Step risk, case-normalised. The extractor emits both `Medium` and `medium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            _ => return None,
        })
    }

    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];
}

/// Legality of the work. The extractor emits `Legal` and `Lawful` for the same
/// thing, and NULL for the majority of contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalStatus {
    Legal,
    Illegal,
    /// Not explicitly criminal but not sanctioned either.
    Grey,
}

impl LegalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legal => "legal",
            Self::Illegal => "illegal",
            Self::Grey => "grey",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "legal" => Self::Legal,
            "illegal" => Self::Illegal,
            "grey" => Self::Grey,
            _ => return None,
        })
    }

    pub const ALL: [Self; 3] = [Self::Legal, Self::Illegal, Self::Grey];
}

/// Classify a contract into its parent category.
///
/// `contract_type` is the extractor's own label and wins whenever it is
/// recognised. `gameplay_loop` is consulted only when the type is absent or
/// unrecognised. An unrecognised pair yields [`ContractCategory::Other`];
/// this function is total and never fails.
pub fn classify(contract_type: Option<&str>, gameplay_loop: Option<&str>) -> ContractCategory {
    if let Some(c) = contract_type.and_then(match_category) {
        return c;
    }
    gameplay_loop
        .and_then(match_category)
        .unwrap_or(ContractCategory::Other)
}

/// Case-normalise a raw step risk. Unrecognised input is `None`, and the raw
/// value is still published alongside, so nothing is lost.
pub fn normalise_risk(raw: Option<&str>) -> Option<Risk> {
    match normalise(raw?).as_str() {
        "low" | "minimal" | "none" => Some(Risk::Low),
        "medium" | "moderate" => Some(Risk::Medium),
        "high" | "extreme" | "very high" => Some(Risk::High),
        _ => None,
    }
}

/// Normalise a raw legal status.
///
/// NULL stays NULL. Absent is NOT the same as legal, and defaulting here would
/// be a silent claim about the 83 of 124 live contracts that say nothing.
pub fn normalise_legal_status(raw: Option<&str>) -> Option<LegalStatus> {
    match normalise(raw?).as_str() {
        "legal" | "lawful" | "licensed" => Some(LegalStatus::Legal),
        "illegal" | "unlawful" | "criminal" => Some(LegalStatus::Illegal),
        "grey" | "gray" | "grey market" | "gray market" | "questionable" => Some(LegalStatus::Grey),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Internals.
// ---------------------------------------------------------------------------

/// Lowercase, and collapse every run of non-alphanumeric characters to a single
/// space. `"SALVAGE"`, `"Salvage"` and `" salvage "` all become `"salvage"`,
/// which is why the alias table needs no case or punctuation variants.
fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.extend(ch.to_lowercase());
        } else {
            pending_space = true;
        }
    }
    out
}

/// Match one free-text label against the alias table.
///
/// Tries the whole normalised string first, then — because `gameplay_loop`
/// carries compound values like `"Mining, Delivery"`, `"Cargo & Hauling"` and
/// `"Hauling / Delivery"` — each separated part in order, so the leading
/// concept wins.
fn match_category(raw: &str) -> Option<ContractCategory> {
    let n = normalise(raw);
    if n.is_empty() {
        return None;
    }
    if let Some(c) = alias(&n) {
        return Some(c);
    }
    // `normalise` has already collapsed ',', '&' and '/' to spaces, so the
    // compound forms are split on whitespace runs against the alias table by
    // progressively shorter prefixes, then by single words.
    let words: Vec<&str> = n.split(' ').filter(|w| !w.is_empty()).collect();
    for len in (1..words.len()).rev() {
        if let Some(c) = alias(&words[..len].join(" ")) {
            return Some(c);
        }
    }
    words.iter().find_map(|w| alias(w))
}

/// The alias table. Covers every `contract_type` and `gameplay_loop` value
/// observed in the live catalogue on 2026-08-06, plus obvious neighbours.
fn alias(s: &str) -> Option<ContractCategory> {
    use ContractCategory as C;
    Some(match s {
        // Bounty work — a named target.
        "bounty hunting" | "bounty" | "bounty hunter" | "apprehension" => C::BountyHunter,

        // Paid combat that is not a named-target bounty.
        "mercenary" | "combat" | "extermination" | "arena combat" | "defend location"
        | "defence" | "defense" | "escort" | "elimination" => C::Mercenary,

        // Cargo movement, including recovering cargo already lost.
        "hauling" | "cargo hauling" | "cargo haul" | "cargo" | "cargo recovery"
        | "cargo retrieval" | "freight" | "logistics" => C::Hauling,

        // Point-to-point item delivery.
        "delivery" | "deliveries" | "cargo delivery" | "courier" | "transport"
        | "transportation" => C::Delivery,

        "salvage" | "reclamation" | "wreck recovery" => C::Salvage,
        "mining" | "extraction" | "prospecting" => C::Mining,

        // Fetching specific items that are not cargo.
        "retrieval" | "collection" | "gathering" | "acquisition" | "fetch" => C::Collection,

        "investigation" | "recon" | "reconnaissance" | "search and rescue" | "scanning" => {
            C::Investigation
        }

        "refueling" | "refuelling" | "repair" | "maintenance" | "servicing" | "rearm" => {
            C::Maintenance
        }

        "certification" | "training" | "combat training" | "combat simulation" | "tutorial"
        | "flight school" => C::Training,

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `contract_type` value present in the live catalogue on
    /// 2026-08-06, with its expected category. `Personal` is the only value
    /// that legitimately lands in `Other`.
    const LIVE_CONTRACT_TYPES: &[(&str, ContractCategory)] = &[
        ("Delivery", ContractCategory::Delivery),
        ("Hauling", ContractCategory::Hauling),
        ("Mercenary", ContractCategory::Mercenary),
        ("Salvage", ContractCategory::Salvage),
        ("Courier", ContractCategory::Delivery),
        ("Bounty Hunting", ContractCategory::BountyHunter),
        ("Cargo Haul", ContractCategory::Hauling),
        ("Retrieval", ContractCategory::Collection),
        ("Cargo Recovery", ContractCategory::Hauling),
        ("Refueling", ContractCategory::Maintenance),
        ("Investigation", ContractCategory::Investigation),
        ("Personal", ContractCategory::Other),
        ("Cargo Retrieval", ContractCategory::Hauling),
        ("Certification", ContractCategory::Training),
        ("Combat", ContractCategory::Mercenary),
        ("SALVAGE", ContractCategory::Salvage),
    ];

    #[test]
    fn every_live_contract_type_classifies_as_specified() {
        for (raw, want) in LIVE_CONTRACT_TYPES {
            assert_eq!(
                classify(Some(raw), None),
                *want,
                "contract_type {raw:?} misclassified"
            );
        }
    }

    #[test]
    fn only_personal_falls_through_to_other() {
        let fell_through: Vec<&str> = LIVE_CONTRACT_TYPES
            .iter()
            .filter(|(raw, _)| classify(Some(raw), None) == ContractCategory::Other)
            .map(|(raw, _)| *raw)
            .collect();
        assert_eq!(fell_through, vec!["Personal"]);
    }

    #[test]
    fn case_and_punctuation_do_not_matter() {
        for raw in ["SALVAGE", "Salvage", "salvage", "  salvage  ", "Salvage!"] {
            assert_eq!(classify(Some(raw), None), ContractCategory::Salvage);
        }
        assert_eq!(
            normalise_risk(Some("Medium")),
            normalise_risk(Some("medium"))
        );
        assert_eq!(
            normalise_legal_status(Some("LEGAL")),
            normalise_legal_status(Some("legal"))
        );
    }

    #[test]
    fn contract_type_wins_over_gameplay_loop() {
        // A salvage contract whose loop says combat is still salvage.
        assert_eq!(
            classify(Some("Salvage"), Some("Combat")),
            ContractCategory::Salvage
        );
    }

    #[test]
    fn gameplay_loop_is_used_when_type_is_absent_or_unrecognised() {
        assert_eq!(
            classify(None, Some("Bounty Hunting")),
            ContractCategory::BountyHunter
        );
        assert_eq!(
            classify(Some("Wat"), Some("Salvage")),
            ContractCategory::Salvage
        );
    }

    #[test]
    fn compound_gameplay_loops_take_the_leading_concept() {
        // Every compound value observed live.
        for (raw, want) in [
            ("Mining, Delivery", ContractCategory::Mining),
            ("Mining & Delivery", ContractCategory::Mining),
            ("Hauling / Delivery", ContractCategory::Hauling),
            ("Cargo & Hauling", ContractCategory::Hauling),
            ("Cargo & Deliveries", ContractCategory::Hauling),
        ] {
            assert_eq!(
                classify(None, Some(raw)),
                want,
                "loop {raw:?} misclassified"
            );
        }
    }

    #[test]
    fn multiword_aliases_beat_their_leading_word() {
        // "combat training" must not be swallowed by "combat" -> Mercenary.
        assert_eq!(
            classify(Some("Combat Training"), None),
            ContractCategory::Training
        );
        assert_eq!(
            classify(Some("Combat Simulation"), None),
            ContractCategory::Training
        );
        assert_eq!(
            classify(Some("Cargo Recovery"), None),
            ContractCategory::Hauling
        );
    }

    #[test]
    fn classify_is_total() {
        for raw in [
            "",
            "   ",
            "!!!",
            "\u{1F680}",
            "something nobody has ever written",
        ] {
            assert_eq!(classify(Some(raw), Some(raw)), ContractCategory::Other);
        }
        assert_eq!(classify(None, None), ContractCategory::Other);
    }

    #[test]
    fn risk_normalises_the_live_case_split() {
        assert_eq!(normalise_risk(Some("medium")), Some(Risk::Medium));
        assert_eq!(normalise_risk(Some("Medium")), Some(Risk::Medium));
        assert_eq!(normalise_risk(Some("high")), Some(Risk::High));
        assert_eq!(normalise_risk(Some("High")), Some(Risk::High));
        assert_eq!(normalise_risk(Some("low")), Some(Risk::Low));
        assert_eq!(normalise_risk(Some("Low")), Some(Risk::Low));
        assert_eq!(normalise_risk(Some("banana")), None);
        assert_eq!(normalise_risk(None), None);
    }

    #[test]
    fn legal_status_folds_lawful_into_legal_and_keeps_null_null() {
        assert_eq!(
            normalise_legal_status(Some("Legal")),
            Some(LegalStatus::Legal)
        );
        assert_eq!(
            normalise_legal_status(Some("Lawful")),
            Some(LegalStatus::Legal)
        );
        assert_eq!(normalise_legal_status(Some("banana")), None);
        // Absent must NOT become `legal` — that would be a silent claim about
        // the 83 of 124 live contracts that say nothing.
        assert_eq!(normalise_legal_status(None), None);
    }

    #[test]
    fn enums_round_trip_through_their_string_form() {
        for c in ContractCategory::ALL {
            assert_eq!(ContractCategory::parse(c.as_str()), Some(c));
        }
        for r in Risk::ALL {
            assert_eq!(Risk::parse(r.as_str()), Some(r));
        }
        for l in LegalStatus::ALL {
            assert_eq!(LegalStatus::parse(l.as_str()), Some(l));
        }
        assert_eq!(ContractCategory::parse("nope"), None);
    }

    #[test]
    fn every_category_is_reachable_from_some_alias() {
        // Guards against a variant that exists but can never be produced.
        // `Other` is excluded: it is the fallthrough, not an alias target.
        for c in ContractCategory::ALL {
            if c == ContractCategory::Other {
                continue;
            }
            let probe = c.as_str().replace('_', " ");
            assert_eq!(
                classify(Some(&probe), None),
                c,
                "category {c:?} is unreachable — no alias produces it"
            );
        }
    }

    #[test]
    fn serde_emits_snake_case() {
        assert_eq!(
            serde_json::to_string(&ContractCategory::BountyHunter).unwrap(),
            "\"bounty_hunter\""
        );
        assert_eq!(serde_json::to_string(&Risk::Medium).unwrap(), "\"medium\"");
        assert_eq!(
            serde_json::to_string(&LegalStatus::Legal).unwrap(),
            "\"legal\""
        );
    }
}
