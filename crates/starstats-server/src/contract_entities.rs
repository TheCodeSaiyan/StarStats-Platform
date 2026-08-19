//! Which KB entities a contract references — pure extraction and
//! normalization.
//!
//! Feeds the `contract_entities` table (migration 0062), which answers
//! both link directions: what a contract touches, and which contracts
//! touch a given KB entity.
//!
//! This module is deliberately free of I/O. Resolving a name to a KB
//! slug needs `reference_registry` and therefore belongs to the store
//! (see `ContractStore::list_by_entity` and friends); deciding *which
//! strings are entities at all* is a property of the extraction and is
//! tested without a database.
//!
//! Kept out of `contracts.rs` because that file is already ~1600 lines
//! and owns a different table.

use crate::contracts::ExtractionReq;

/// One KB entity a contract references, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityRef {
    /// `location` | `item` | `vehicle` | `weapon`.
    pub kind: String,
    /// Verbatim as captured — this is what renders when the entity does
    /// not resolve to a KB entry.
    pub raw_value: String,
    /// Primary-key component; see `normalize_value`.
    pub value_norm: String,
}

/// Lower-case, trim, and collapse internal whitespace.
///
/// This is the value the table's primary key is built on, so two
/// spellings that differ only in spacing must land on one row —
/// "Glaciem  Ring" and "glaciem ring" are the same place.
pub fn normalize_value(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Append an entity, skipping blanks and duplicates.
///
/// De-duplication is on `(kind, value_norm)` rather than `raw_value`
/// because that is the table's primary key: emitting both "Glaciem Ring"
/// and "glaciem  ring" would make the INSERT self-collide.
fn push(out: &mut Vec<EntityRef>, kind: &str, raw: &str) {
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    let value_norm = normalize_value(raw);
    if value_norm.is_empty() {
        return;
    }
    if out
        .iter()
        .any(|e| e.kind == kind && e.value_norm == value_norm)
    {
        return;
    }
    out.push(EntityRef {
        kind: kind.to_string(),
        raw_value: raw.to_string(),
        value_norm,
    });
}

/// Every KB entity this extraction references, de-duplicated on
/// `(kind, normalized value)`.
///
/// Items come from *every* step, not just the first. The promoted
/// `contracts.required_item` column stays first-step-only because it
/// backs a single-line list display; an index of "which contracts need
/// this item" that only saw step one would silently miss most of them.
pub fn extract_entities(extraction: &ExtractionReq) -> Vec<EntityRef> {
    let mut out = Vec::new();

    for step in &extraction.steps {
        // Canonical names the extractor identified. These are what can
        // resolve: the neighbouring `location` / `required_item` fields
        // hold the contract's own phrasing ("Caterpillar wreck site near
        // microTech", "Requested research resource (quantity 15)"),
        // which names nothing the registry holds. Measured on the first
        // 10 published contracts — indexing that prose would have
        // produced an index that resolves to almost nothing.
        for e in &step.entities {
            let (Some(kind), Some(name)) = (&e.kind, &e.name) else {
                continue;
            };
            let kind = kind.trim().to_lowercase();
            if !kind.is_empty() {
                push(&mut out, &kind, name);
            }
        }
    }

    // DETAILS lines such as "LAST KNOWN LOCATION" usually carry a bare
    // place name already, and often one the steps never mention.
    for attr in &extraction.contract.attributes {
        let label = attr.label.as_deref().unwrap_or("").to_uppercase();
        if label.contains("LOCATION") {
            if let Some(v) = &attr.value {
                push(&mut out, "location", v);
            }
        }
    }

    out
}

/// A stored entity row, after resolution against `reference_registry`.
///
/// `ref_slug`/`ref_category` are `None` whenever resolution was not
/// unambiguous — no match, several matches, or a matched registry row
/// whose own slug predates migration 0038.
///
/// Use `ref_match_count` to tell those apart: several matches links to a
/// knowledge-base search (we know entries exist), no match renders as
/// plain text (we know nothing, so a search link is a dead end).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct EntityRow {
    pub kind: String,
    pub raw_value: String,
    pub ref_slug: Option<String>,
    pub ref_category: Option<String>,
    /// How many registry rows this name matched.
    ///
    /// Distinguishes the two cases that both leave `ref_slug` empty:
    /// `0` means the knowledge base holds nothing by this name, so there
    /// is nowhere to send anyone; `>1` means it holds several and we
    /// cannot say which — the case that links to a search rather than
    /// guessing one.
    pub ref_match_count: i64,
}

/// How many catalogue rows carry a given contract name.
///
/// `canonical_id` is `Some` only when `match_count == 1`. `display_name`
/// is deliberately non-unique, so any other count must not name a
/// winner — the caller links to the filtered candidate list instead.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct NameResolution {
    /// The normalized name that was looked up.
    pub name: String,
    pub match_count: i64,
    pub canonical_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extraction_with(json: serde_json::Value) -> ExtractionReq {
        serde_json::from_value(json).expect("extraction parses")
    }

    #[test]
    fn normalize_collapses_case_and_whitespace() {
        assert_eq!(normalize_value("  Glaciem   Ring "), "glaciem ring");
        assert_eq!(normalize_value("MG Scrip"), "mg scrip");
    }

    #[test]
    fn indexes_canonical_names_not_the_descriptive_prose() {
        // The real shape of the data: `location` is a sentence, and the
        // names worth indexing are in `entities`. Indexing the prose
        // would build an index that resolves to nothing.
        let ex = extraction_with(serde_json::json!({
            "contract": {},
            "steps": [{
                "order": 1, "summary": "Salvage",
                "location": "Caterpillar wreck site near microTech",
                "required_item": "Requested research resource (quantity 15)",
                "entities": [
                    { "kind": "vehicle",  "name": "Caterpillar" },
                    { "kind": "location", "name": "microTech" }
                ]
            }]
        }));
        let got = extract_entities(&ex);

        let pairs: Vec<(&str, &str)> = got
            .iter()
            .map(|e| (e.kind.as_str(), e.raw_value.as_str()))
            .collect();
        assert!(pairs.contains(&("vehicle", "Caterpillar")));
        assert!(pairs.contains(&("location", "microTech")));
        // One phrase, two entities of DIFFERENT kinds — the reason this
        // is a list rather than a cleaned-up scalar field.
        assert_eq!(got.len(), 2);
        // The prose itself must never become an entity.
        assert!(!pairs
            .iter()
            .any(|(_, v)| v.contains("wreck site") || v.contains("quantity")));
    }

    #[test]
    fn a_step_naming_nothing_contributes_nothing() {
        // "Requested research resource (quantity 15)" names no entity;
        // the extractor is told to leave `entities` empty rather than
        // invent one, and an empty list must stay empty.
        let ex = extraction_with(serde_json::json!({
            "contract": {},
            "steps": [{ "order": 1, "summary": "Fetch",
                        "required_item": "Requested research resource (quantity 15)",
                        "entities": [] }]
        }));
        assert!(extract_entities(&ex).is_empty());
    }

    #[test]
    fn takes_location_attributes_which_are_already_bare_names() {
        let ex = extraction_with(serde_json::json!({
            "contract": { "attributes": [
                { "label": "LAST KNOWN LOCATION", "value": "Glaciem Ring" },
                { "label": "RISK ASSESSMENT", "value": "Medium" } ] },
            "steps": []
        }));
        let got = extract_entities(&ex);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "location");
        assert_eq!(got[0].raw_value, "Glaciem Ring");
    }

    #[test]
    fn deduplicates_on_the_normalized_value() {
        // The table's PK is (canonical_id, kind, value_norm), so two
        // spellings of one name would make the INSERT self-collide.
        let ex = extraction_with(serde_json::json!({
            "contract": { "attributes": [
                { "label": "LAST KNOWN LOCATION", "value": "glaciem ring" } ] },
            "steps": [{ "order": 1, "summary": "Go",
                        "entities": [{ "kind": "location", "name": "Glaciem  Ring" }] }]
        }));
        assert_eq!(extract_entities(&ex).len(), 1, "one place, one row");
    }

    #[test]
    fn the_same_name_under_two_kinds_is_kept_separately() {
        // De-duplication is per kind: a ship and a place may share a
        // name, and collapsing them would lose one.
        let ex = extraction_with(serde_json::json!({
            "contract": {},
            "steps": [{ "order": 1, "summary": "Go", "entities": [
                { "kind": "vehicle",  "name": "Stanton" },
                { "kind": "location", "name": "Stanton" } ] }]
        }));
        assert_eq!(extract_entities(&ex).len(), 2);
    }

    #[test]
    fn drops_entries_missing_a_kind_or_a_name() {
        let ex = extraction_with(serde_json::json!({
            "contract": {},
            "steps": [{ "order": 1, "summary": "Go", "entities": [
                { "kind": "location", "name": "   " },
                { "kind": "   ",      "name": "Somewhere" },
                { "name": "No Kind" },
                { "kind": "location", "name": "microTech" } ] }]
        }));
        let got = extract_entities(&ex);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].raw_value, "microTech");
    }
}
