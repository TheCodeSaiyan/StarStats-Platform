//! Built-in `BurstRule` definitions for the four spammy event types
//! observed in real Game.log captures:
//!
//! * `<AttachmentReceived>` — spawn-restore burst (20+ items in one ms
//!   when the player rezzes at a station). Anchored on the `body_*`
//!   attachment, but in practice every member is an `AttachmentReceived`
//!   with the `[Inventory]` tag, so we don't require the body_ prefix
//!   on the anchor — any AttachmentReceived followed by 4+ more in a
//!   tight window is a loadout restoration.
//!
//! * `<StatObjLoad 0x800 Format>` — terrain shower during planet entry,
//!   exit, and atmospheric transition. Single-millisecond bursts of 30+.
//!
//! * `<SHUDEvent_OnNotification>` — jurisdiction banner stutters when
//!   crossing zone boundaries. Less spammy than the others but visually
//!   distracting on the timeline (3-5 banners per crossing).
//!
//! * `<VehicleStowed>` — fires once per ship in a hangar when the
//!   player enters/exits the lobby. Bursts of 3-15 depending on
//!   account hangar size.
//!
//! Tuning rationale (`min_burst_size`, `max_member_gap`):
//!   * `min_burst_size: 3` is the floor for collapse — short legitimate
//!     pairs (e.g. equip + unequip) shouldn't be hidden behind a summary.
//!   * `max_member_gap: 1` for AttachmentReceived (lets InventoryManagement
//!     events between consecutive attachments pass without breaking the
//!     burst); `0` for the others (their members are strictly contiguous
//!     in observed captures).
//!
//! These rules ship as Rust constants for the bootstrap path. A future
//! slice will move them to the server-managed
//! `/v1/parser-definitions` manifest so thresholds can be tuned without
//! a tray release; the data types are already serialisable for that.

use starstats_core::templates::{BurstRule, StepMatch};

/// The canonical rule id for the loadout-restore burst.
///
/// Used both in `builtin_burst_rules()` (to produce the rule) and in
/// `gamelog.rs` / `commands.rs` (to identify a matched burst and set
/// `kind = "loadout_restore"`). Defining it here as a single source
/// of truth prevents the rule-id string from drifting between the
/// definition site and the two consumption sites (the live-tail path
/// and the re-parse path).
pub const LOADOUT_RESTORE_BURST_RULE_ID: &str = "loadout_restore_burst";

/// Build the four built-in burst rules. Allocates fresh `String`s
/// per call — cheap, and matches the manifest-served path's
/// allocation model, so tests that round-trip rules through JSON
/// hit the same code shape.
pub fn builtin_burst_rules() -> Vec<BurstRule> {
    vec![
        BurstRule {
            id: LOADOUT_RESTORE_BURST_RULE_ID.to_string(),
            anchor: StepMatch::EventName {
                name: "AttachmentReceived".to_string(),
            },
            member: StepMatch::EventName {
                name: "AttachmentReceived".to_string(),
            },
            tags: vec!["Inventory".to_string()],
            min_burst_size: 3,
            // One InventoryManagement line between consecutive
            // attachments is normal; tolerate it.
            max_member_gap: 1,
        },
        BurstRule {
            id: "terrain_load_burst".to_string(),
            anchor: StepMatch::EventName {
                name: "StatObjLoad 0x800 Format".to_string(),
            },
            member: StepMatch::EventName {
                name: "StatObjLoad 0x800 Format".to_string(),
            },
            tags: vec!["CoreTech".to_string()],
            min_burst_size: 5,
            max_member_gap: 0,
        },
        BurstRule {
            id: "hud_notification_burst".to_string(),
            anchor: StepMatch::EventName {
                name: "SHUDEvent_OnNotification".to_string(),
            },
            member: StepMatch::EventName {
                name: "SHUDEvent_OnNotification".to_string(),
            },
            tags: vec![],
            // Notifications come in 3-5 stutters when the user crosses
            // jurisdictions. Lower threshold = collapse more aggressively.
            min_burst_size: 3,
            max_member_gap: 0,
        },
        BurstRule {
            id: "vehicle_stowed_burst".to_string(),
            anchor: StepMatch::EventName {
                name: "VehicleStowed".to_string(),
            },
            member: StepMatch::EventName {
                name: "VehicleStowed".to_string(),
            },
            tags: vec![],
            min_burst_size: 3,
            max_member_gap: 0,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_rules_count_matches_documented_set() {
        let rules = builtin_burst_rules();
        assert_eq!(rules.len(), 4);
        let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"loadout_restore_burst"));
        assert!(ids.contains(&"terrain_load_burst"));
        assert!(ids.contains(&"hud_notification_burst"));
        assert!(ids.contains(&"vehicle_stowed_burst"));
    }

    #[test]
    fn builtin_rules_have_unique_ids() {
        let rules = builtin_burst_rules();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for rule in &rules {
            assert!(
                seen.insert(rule.id.as_str()),
                "duplicate rule id: {}",
                rule.id
            );
        }
    }

    #[test]
    fn builtin_rules_round_trip_through_json() {
        // Future slice will fetch these from the parser-definitions
        // manifest; lock the JSON shape now so the manifest format is
        // settled before the migration.
        let rules = builtin_burst_rules();
        let json = serde_json::to_string(&rules).expect("serialise");
        let parsed: Vec<BurstRule> = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(parsed, rules);
    }

    #[test]
    fn builtin_loadout_rule_min_size_is_at_least_three() {
        // Floor protects against accidentally collapsing
        // legitimate two-event pairs (equip + immediate unequip).
        let rules = builtin_burst_rules();
        let loadout = rules
            .iter()
            .find(|r| r.id == "loadout_restore_burst")
            .expect("loadout rule present");
        assert!(loadout.min_burst_size >= 3);
    }
}
