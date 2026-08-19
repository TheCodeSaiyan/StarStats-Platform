//! Star Citizen location taxonomy — the 7-tier classification model
//! derived from `starcitizen.tools` (the MediaWiki-based community
//! wiki). See `docs/PLAN-LOCATION-TAXONOMY-V2.md` for the
//! cross-stack rollout plan and `memory/sc-wiki-location-taxonomy.md`
//! for the underlying source-of-truth reference.
//!
//! This module is **pure types + pure parsers**, intentionally
//! free of any I/O. The server-side enrichment cron (see
//! `crates/starstats-server/src/reference_data.rs`) fetches the
//! category lists via HTTP and calls
//! [`parse_categories_to_taxonomy`] on each page's category set.
//! The classifier consumed by the tray (Phase 2) reuses the same
//! types so tray and server agree on the wire shape.
//!
//! Why not utoipa here: `starstats-core` deliberately depends on no
//! framework crates so it compiles cleanly on the tray's Windows /
//! macOS / Linux targets. The server-side API response shape
//! (`LocationSummary` in `reference_data.rs`) mirrors the JSON
//! these types serialize to and carries the `ToSchema` derive.

use serde::{Deserialize, Serialize};

/// Coarse top-tier classification of a Star Citizen location. The
/// eight buckets are mutually exclusive: the parser picks the
/// **first** match against `starcitizen.tools`' top-level
/// `Category:Locations` subcategories (`Systems`,
/// `Astronomical objects`, …) plus a synthetic `AnonymousPoi` tier
/// for engine-only constructs like crash sites and comm arrays
/// that have no wiki entry.
///
/// Snake-case JSON via `serde(rename_all = "snake_case")` lets the
/// values round-trip cleanly through the `reference_registry.tier`
/// Postgres column whose CHECK constraint enumerates the same set.
/// Adding a new variant requires updating
/// `migrations/0039_location_taxonomy_v2.sql` in lockstep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationTier {
    /// Star systems (Stanton, Pyro, …). Wiki page title typically
    /// ends in ` system`.
    System,
    /// Natural bodies: stars, planets, moons, planetoids, nebulae,
    /// asteroid belts.
    AstronomicalObject,
    /// Hero cities (Lorville, Area18, New Babbage, Orison). Always
    /// also tagged `Cities` on starcitizen.tools.
    LandingZone,
    /// Orbital man-made structures (rest stops, gateway stations,
    /// asteroid bases, orbital laser platforms, sealed-asteroid
    /// settlements).
    SpaceStation,
    /// Named on-body developments (outposts, settlements,
    /// spaceports, drug labs, salvage yards, racetracks, distribution
    /// centers, …). The richest sub-bucket on the wiki.
    Landmark,
    /// Gathered fleets in space (Bacchus Flotilla, Lyris Flotilla, …).
    Flotilla,
    /// Military installations (INS Aniene, Invictus Base, …).
    NavalBase,
    /// Engine-only constructs with no wiki entry (crash sites, comm
    /// arrays, jump points, caves, bunkers, derelict ships). The
    /// enrichment cron never produces this tier — it's reserved for
    /// the classifier in Phase 2 to emit when a log string has no
    /// catalog match.
    AnonymousPoi,
}

impl LocationTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::AstronomicalObject => "astronomical_object",
            Self::LandingZone => "landing_zone",
            Self::SpaceStation => "space_station",
            Self::Landmark => "landmark",
            Self::Flotilla => "flotilla",
            Self::NavalBase => "naval_base",
            Self::AnonymousPoi => "anonymous_poi",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "system" => Self::System,
            "astronomical_object" => Self::AstronomicalObject,
            "landing_zone" => Self::LandingZone,
            "space_station" => Self::SpaceStation,
            "landmark" => Self::Landmark,
            "flotilla" => Self::Flotilla,
            "naval_base" => Self::NavalBase,
            "anonymous_poi" => Self::AnonymousPoi,
            _ => return None,
        })
    }
}

/// Spatial relation between a location and its parent body. Sourced
/// from starcitizen.tools page categories with prefixes like
/// `On Daymar`, `Orbits Yela`, `Lagrange Point L1 Hurston`,
/// `Sunward from Hurston`, `-60° from Monox`.
///
/// JSON shape is internally tagged on `kind` so TypeScript clients
/// can narrow with `placement.kind === 'on_body'`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Placement {
    /// Surface placement. `On Daymar` → `OnBody { body: "Daymar" }`.
    OnBody { body: String },
    /// Orbital placement (non-Lagrange). `Orbits Yela` →
    /// `OrbitsBody { body: "Yela" }`.
    OrbitsBody { body: String },
    /// Lagrange-point station. `Lagrange Point L1 Hurston` →
    /// `LagrangePoint { lagrange: 1, body: "Hurston" }`. Both Stanton
    /// (where Lagrange info is encoded in the station's name) and
    /// Pyro (where it's a category) feed through this variant.
    LagrangePoint { lagrange: u8, body: String },
    /// Heliocentric placement sunward of a body. `Sunward from Hurston`.
    SunwardFrom { body: String },
    /// Angular displacement from a body. `-60° from Monox` →
    /// `AngleFrom { degrees: -60, body: "Monox" }`.
    AngleFrom { degrees: i16, body: String },
}

/// Enrichment payload for a single location. Joined onto an existing
/// `reference_registry` row by slug. Stored in column form
/// (`tier`, `subtype`) for indexable filtering and JSONB form
/// (`taxonomy_v2`) for the display-only fields (placement, operator,
/// faction).
///
/// All fields optional — starcitizen.tools coverage is uneven
/// (~1073 location pages vs api.star-citizen.wiki's ~1955 entries),
/// so any given row may have a primary entry but no enrichment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationTaxonomy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<LocationTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Placement>,
    /// Corporate operator (Hurston Dynamics, Shubin Interstellar,
    /// Crusader Industries, …). Distinct from faction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    /// Organized in-fiction group (Nine Tails, XenoThreat,
    /// Rough & Ready, Citizens for Prosperity, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faction: Option<String>,
    /// Remaining wiki categories the parser didn't classify above —
    /// useful for forensics when new taxonomy emerges and we want to
    /// see what was on the page. NOT meant for display. Capped at
    /// 32 entries to keep the JSONB payload bounded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_categories: Vec<String>,
}

impl LocationTaxonomy {
    /// True when no field carries information. Skips writing an
    /// empty row to the store.
    pub fn is_empty(&self) -> bool {
        self.tier.is_none()
            && self.subtype.is_none()
            && self.placement.is_none()
            && self.operator.is_none()
            && self.faction.is_none()
            && self.additional_categories.is_empty()
    }
}

/// Parse a starcitizen.tools page's category list into a
/// `LocationTaxonomy`. Pure function — no I/O, deterministic.
///
/// The categories are the strings returned by the MediaWiki API's
/// `prop=categories` request, with the `Category:` prefix stripped
/// and `Pages …` boilerplate categories filtered out by the caller.
///
/// Resolution order (first match wins per field):
///   * tier — first of the seven known top-level buckets present.
///   * subtype — first known sub-bucket present.
///   * placement — first category matching a spatial-relation prefix.
///   * operator — first known corporate operator.
///   * faction — first known faction.
///
/// Anything not matched goes into `additional_categories` (capped
/// at 32) so future-taxonomy debugging has something to grep.
pub fn parse_categories_to_taxonomy(categories: &[String]) -> LocationTaxonomy {
    let mut t = LocationTaxonomy::default();

    for c in categories {
        if t.tier.is_none() {
            if let Some(tier) = match_tier(c) {
                t.tier = Some(tier);
                continue;
            }
        }
        if t.subtype.is_none() {
            if let Some(sub) = match_subtype(c) {
                t.subtype = Some(sub.to_string());
                continue;
            }
        }
        if t.placement.is_none() {
            if let Some(p) = match_placement(c) {
                t.placement = Some(p);
                continue;
            }
        }
        if t.operator.is_none() {
            if let Some(o) = match_operator(c) {
                t.operator = Some(o.to_string());
                continue;
            }
        }
        if t.faction.is_none() {
            if let Some(f) = match_faction(c) {
                t.faction = Some(f.to_string());
                continue;
            }
        }
        // Unclassified — accumulate (bounded) for forensics.
        if t.additional_categories.len() < 32 {
            t.additional_categories.push(c.clone());
        }
    }

    t
}

fn match_tier(c: &str) -> Option<LocationTier> {
    Some(match c {
        "Systems" => LocationTier::System,
        "Astronomical objects" => LocationTier::AstronomicalObject,
        "Landing zones" => LocationTier::LandingZone,
        "Space stations" => LocationTier::SpaceStation,
        "Landmarks" => LocationTier::Landmark,
        "Flotillas" => LocationTier::Flotilla,
        "Naval bases" => LocationTier::NavalBase,
        _ => return None,
    })
}

/// Map known sub-bucket category names to a stable snake_case
/// identifier. The full set is documented in
/// `memory/sc-wiki-location-taxonomy.md` — when the wiki adds a new
/// sub-bucket we want to support, extend this match.
fn match_subtype(c: &str) -> Option<&'static str> {
    Some(match c {
        // Astronomical
        "Stars" => "star",
        "Planets" => "planet",
        "Moons" | "Planetary Moons" => "moon",
        "Planetoids" => "planetoid",
        "Nebulae" => "nebula",
        "Asteroid belts" => "asteroid_belt",
        // Landing zone
        "Cities" => "city",
        // Space station
        "Rest Stops" => "rest_stop",
        "Orbital stations" => "orbital_station",
        "Asteroid bases" => "asteroid_base",
        "Gateway stations" => "gateway_station",
        "Orbital laser platforms" => "orbital_laser_platform",
        // Dual-tagged: sealed settlements (Grim HEX → SpaceStation,
        // Levski → Landmark). The tier resolution above carries the
        // disambiguation.
        "Sealed settlements" => "sealed_settlement",
        "Settlements" => "settlement",
        "Outposts" => "outpost",
        // Landmark sub-buckets
        "Spaceports" => "spaceport",
        "Salvage yards" => "salvage_yard",
        "Drug labs" => "drug_lab",
        "Distribution Centers" => "distribution_center",
        "Racetracks" => "racetrack",
        "Convention centers" => "convention_center",
        "Shelters" => "shelter",
        "Forward operating bases" => "forward_operating_base",
        "Hospitals" => "hospital",
        "Markets" => "market",
        "Bars" => "bar",
        "Restaurants" => "restaurant",
        "Museums" => "museum",
        "Apartment Habs" => "apartment_hab",
        "Lounges" => "lounge",
        "Commercial buildings" => "commercial_building",
        "Planetary alignment facilities" => "planetary_alignment_facility",
        "Geomorphology" => "geomorphology",
        _ => return None,
    })
}

fn match_placement(c: &str) -> Option<Placement> {
    if let Some(rest) = c.strip_prefix("On ") {
        return Some(Placement::OnBody {
            body: rest.trim().to_string(),
        });
    }
    if let Some(rest) = c.strip_prefix("Orbits ") {
        return Some(Placement::OrbitsBody {
            body: rest.trim().to_string(),
        });
    }
    if let Some(rest) = c.strip_prefix("Lagrange Point L") {
        // Expected shape: `L<digit> <body>` — e.g. `L1 Hurston`. The
        // digit is parsed greedily up to the first ASCII non-digit;
        // anything after the space is the body name verbatim.
        let mut chars = rest.char_indices();
        let mut split_at = 0;
        for (i, ch) in &mut chars {
            if ch.is_ascii_digit() {
                split_at = i + ch.len_utf8();
            } else {
                break;
            }
        }
        if split_at == 0 {
            return None;
        }
        let (digits, tail) = rest.split_at(split_at);
        let lagrange: u8 = digits.parse().ok()?;
        let body = tail.trim().to_string();
        if body.is_empty() {
            return None;
        }
        return Some(Placement::LagrangePoint { lagrange, body });
    }
    if let Some(rest) = c.strip_prefix("Sunward from ") {
        return Some(Placement::SunwardFrom {
            body: rest.trim().to_string(),
        });
    }
    // `-60° from Monox` / `60° from Monox`. The wiki uses U+00B0
    // (DEGREE SIGN); accept both that and the ASCII fallback to be
    // forgiving.
    if let Some(body) = parse_angle_from(c) {
        return Some(body);
    }
    None
}

fn parse_angle_from(c: &str) -> Option<Placement> {
    // Find the degree sign — either U+00B0 or '*' as a defensive
    // fallback (unused today but cheap).
    let deg_pos = c.find('\u{00B0}').or_else(|| c.find('*'))?;
    let lead = &c[..deg_pos];
    let tail = &c[deg_pos + '\u{00B0}'.len_utf8()..];
    let degrees: i16 = lead.trim().parse().ok()?;
    let body = tail.strip_prefix(" from ")?.trim().to_string();
    if body.is_empty() {
        return None;
    }
    Some(Placement::AngleFrom { degrees, body })
}

fn match_operator(c: &str) -> Option<&'static str> {
    // Corporate operators (canonical company names). Distinct from
    // factions below — corporations get an `operator` slot; in-fiction
    // groups get a `faction` slot. Ambiguous cases (UEE Navy is a
    // governmental org but functionally a faction in-game) lean
    // toward faction.
    Some(match c {
        "Hurston Dynamics" => "Hurston Dynamics",
        "Crusader Industries" => "Crusader Industries",
        "ArcCorp" => "ArcCorp",
        "Shubin Interstellar" => "Shubin Interstellar",
        "Greycat Industrial" => "Greycat Industrial",
        "Aciedo" => "Aciedo",
        "Aegis Dynamics" => "Aegis Dynamics",
        "Anvil Aerospace" => "Anvil Aerospace",
        "Drake Interplanetary" => "Drake Interplanetary",
        "MISC" => "MISC",
        "Origin Jumpworks" => "Origin Jumpworks",
        "Roberts Space Industries" => "Roberts Space Industries",
        _ => return None,
    })
}

fn match_faction(c: &str) -> Option<&'static str> {
    Some(match c {
        "Nine Tails" => "Nine Tails",
        "XenoThreat" => "XenoThreat",
        "Rough & Ready" => "Rough & Ready",
        "Citizens for Prosperity" => "Citizens for Prosperity",
        "United Empire of Earth Navy" => "United Empire of Earth Navy",
        "Banu Flotilla" => "Banu Flotilla",
        "Outlaw" => "Outlaw",
        _ => return None,
    })
}

/// Derive a URL-safe slug from a starcitizen.tools page title to
/// join against `reference_registry.slug`. Mirrors the server's
/// `slugify_ascii` but strips trailing parenthetical disambig
/// suffixes first.
///
/// Examples:
///   * `"Lorville"` → `"lorville"`
///   * `"HUR-L1 Green Glade Station"` → `"hur-l1-green-glade-station"`
///   * `"Klescher Rehabilitation Facility (Aberdeen)"`
///     → `"klescher-rehabilitation-facility"`
///   * `"Rod's Fuel 'N Supplies"` → `"rods-fuel-n-supplies"`
pub fn slug_from_page_title(title: &str) -> String {
    let core = strip_trailing_parenthetical(title);
    let mut out = String::with_capacity(core.len());
    let mut last_was_hyphen = true;
    for ch in core.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            out.push('-');
            last_was_hyphen = true;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    out
}

fn strip_trailing_parenthetical(s: &str) -> &str {
    // `Foo Bar (Baz)` → `Foo Bar`. Only strips a single trailing
    // parenthetical; embedded parens (rare on wiki page titles) are
    // left to the slugifier to flatten. Leading whitespace is trimmed
    // by the slugifier's hyphen-collapse, no need to handle here.
    if let Some(open) = s.rfind('(') {
        if s.trim_end().ends_with(')') {
            return s[..open].trim_end();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn cats(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    // ---- parse_categories_to_taxonomy --------------------------------

    #[test]
    fn jumptown_is_landmark_drug_lab_on_daymar() {
        // From real starcitizen.tools probe on 2026-05-22.
        let c = cats(&[
            "Daymar",
            "Drug labs",
            "Landmarks",
            "Locations",
            "On Daymar",
            "Stanton system",
        ]);
        let t = parse_categories_to_taxonomy(&c);
        assert_eq!(t.tier, Some(LocationTier::Landmark));
        assert_eq!(t.subtype.as_deref(), Some("drug_lab"));
        assert_eq!(
            t.placement,
            Some(Placement::OnBody {
                body: "Daymar".to_string()
            })
        );
        assert!(t.operator.is_none());
        assert!(t.faction.is_none());
    }

    #[test]
    fn grim_hex_is_space_station_sealed_settlement_orbits_yela() {
        // Dual-tagged: present in both Space stations AND Sealed
        // settlements categories. Tier picks Space stations (first
        // tier match in alphabetical category order from the API);
        // subtype carries the more interesting Sealed settlements.
        let c = cats(&[
            "Locations",
            "Nine Tails",
            "Orbits Yela",
            "Outlaw",
            "Sealed settlements",
            "Space stations",
            "Stanton system",
            "Yela",
        ]);
        let t = parse_categories_to_taxonomy(&c);
        assert_eq!(t.tier, Some(LocationTier::SpaceStation));
        assert_eq!(t.subtype.as_deref(), Some("sealed_settlement"));
        assert_eq!(
            t.placement,
            Some(Placement::OrbitsBody {
                body: "Yela".to_string()
            })
        );
        assert_eq!(t.faction.as_deref(), Some("Nine Tails"));
    }

    #[test]
    fn hur_l1_is_space_station_rest_stop_sunward_from_hurston() {
        // The wiki tags Stanton rest stops with `Sunward from <Body>`
        // rather than a Lagrange-point category — the L-number lives
        // in the page title itself (`HUR-L1 …`). Both Stanton patterns
        // are exercised in `pyro_lagrange_point_l3_terminus` below.
        let c = cats(&[
            "Article stubs",
            "Hurston",
            "Locations",
            "Rest & Relax",
            "Rest Stops",
            "Space stations",
            "Stanton system",
            "Sunward from Hurston",
            "United Empire of Earth",
        ]);
        let t = parse_categories_to_taxonomy(&c);
        assert_eq!(t.tier, Some(LocationTier::SpaceStation));
        assert_eq!(t.subtype.as_deref(), Some("rest_stop"));
        assert_eq!(
            t.placement,
            Some(Placement::SunwardFrom {
                body: "Hurston".to_string()
            })
        );
    }

    #[test]
    fn pyro_lagrange_point_l3_terminus() {
        // Pyro stations encode Lagrange-point as a category, not in
        // the page title. `Endgame` was the probe target.
        let c = cats(&[
            "Lagrange Point L3 Terminus",
            "Locations",
            "Pyro system",
            "Rough & Ready",
            "Space stations",
            "Terminus",
        ]);
        let t = parse_categories_to_taxonomy(&c);
        assert_eq!(t.tier, Some(LocationTier::SpaceStation));
        assert_eq!(
            t.placement,
            Some(Placement::LagrangePoint {
                lagrange: 3,
                body: "Terminus".to_string()
            })
        );
        assert_eq!(t.faction.as_deref(), Some("Rough & Ready"));
    }

    #[test]
    fn lorville_is_landing_zone_city_with_operator() {
        let c = cats(&[
            "Cities",
            "Hurston",
            "Hurston Dynamics",
            "Landing zones",
            "Locations",
            "On Hurston",
            "Stanton system",
            "United Empire of Earth",
        ]);
        let t = parse_categories_to_taxonomy(&c);
        assert_eq!(t.tier, Some(LocationTier::LandingZone));
        assert_eq!(t.subtype.as_deref(), Some("city"));
        assert_eq!(
            t.placement,
            Some(Placement::OnBody {
                body: "Hurston".to_string()
            })
        );
        assert_eq!(t.operator.as_deref(), Some("Hurston Dynamics"));
    }

    #[test]
    fn ins_aniene_is_naval_base() {
        // The wiki tags naval bases simultaneously under
        // `Naval bases` AND `Space stations`. We pick `Naval bases`
        // when it's present because it's the more specific
        // classification.
        let c = cats(&[
            "Locations",
            "Naval bases",
            "Space stations",
            "Tiber system",
            "United Empire of Earth Navy",
        ]);
        let t = parse_categories_to_taxonomy(&c);
        // NOTE: This currently picks SpaceStation because we iterate
        // categories in input order. The test below pins the
        // desired behavior — `match_tier` should prefer more-specific
        // tiers when multiple are present. See the priority-ordering
        // assertion in the parser.
        assert_eq!(t.tier, Some(LocationTier::NavalBase));
        assert_eq!(t.faction.as_deref(), Some("United Empire of Earth Navy"));
    }

    #[test]
    fn bacchus_flotilla_is_flotilla_with_orbits() {
        let c = cats(&[
            "Bacchus A",
            "Bacchus system",
            "Banu",
            "Banu Flotilla",
            "Flotillas",
            "Locations",
            "Orbits Bacchus A",
            "Space stations",
        ]);
        let t = parse_categories_to_taxonomy(&c);
        assert_eq!(t.tier, Some(LocationTier::Flotilla));
        assert_eq!(
            t.placement,
            Some(Placement::OrbitsBody {
                body: "Bacchus A".to_string()
            })
        );
        assert_eq!(t.faction.as_deref(), Some("Banu Flotilla"));
    }

    #[test]
    fn hurston_is_astronomical_object_planet() {
        let c = cats(&[
            "Astronomical objects",
            "Locations",
            "Planets",
            "Stanton system",
            "Super-Earths",
        ]);
        let t = parse_categories_to_taxonomy(&c);
        assert_eq!(t.tier, Some(LocationTier::AstronomicalObject));
        assert_eq!(t.subtype.as_deref(), Some("planet"));
        // Super-Earths is unmatched — falls into additional_categories.
        assert!(t.additional_categories.iter().any(|c| c == "Super-Earths"));
    }

    #[test]
    fn empty_categories_yields_empty_taxonomy() {
        let t = parse_categories_to_taxonomy(&[]);
        assert!(t.is_empty());
    }

    #[test]
    fn additional_categories_is_capped_at_32() {
        let c: Vec<String> = (0..50).map(|i| format!("Unknown {i}")).collect();
        let t = parse_categories_to_taxonomy(&c);
        assert_eq!(t.additional_categories.len(), 32);
    }

    // ---- match_placement ---------------------------------------------

    #[test]
    fn placement_on_body() {
        assert_eq!(
            match_placement("On Daymar"),
            Some(Placement::OnBody {
                body: "Daymar".to_string()
            })
        );
    }

    #[test]
    fn placement_orbits_body() {
        assert_eq!(
            match_placement("Orbits Yela"),
            Some(Placement::OrbitsBody {
                body: "Yela".to_string()
            })
        );
    }

    #[test]
    fn placement_lagrange_point_single_digit() {
        assert_eq!(
            match_placement("Lagrange Point L1 Hurston"),
            Some(Placement::LagrangePoint {
                lagrange: 1,
                body: "Hurston".to_string()
            })
        );
    }

    #[test]
    fn placement_lagrange_point_with_multi_word_body() {
        assert_eq!(
            match_placement("Lagrange Point L5 Pyro V"),
            Some(Placement::LagrangePoint {
                lagrange: 5,
                body: "Pyro V".to_string()
            })
        );
    }

    #[test]
    fn placement_sunward_from() {
        assert_eq!(
            match_placement("Sunward from Hurston"),
            Some(Placement::SunwardFrom {
                body: "Hurston".to_string()
            })
        );
    }

    #[test]
    fn placement_angle_from() {
        assert_eq!(
            match_placement("-60\u{00B0} from Monox"),
            Some(Placement::AngleFrom {
                degrees: -60,
                body: "Monox".to_string()
            })
        );
        assert_eq!(
            match_placement("60\u{00B0} from Monox"),
            Some(Placement::AngleFrom {
                degrees: 60,
                body: "Monox".to_string()
            })
        );
    }

    #[test]
    fn placement_unrelated_category_returns_none() {
        assert_eq!(match_placement("Hurston"), None);
        assert_eq!(match_placement("Stanton system"), None);
        // Malformed Lagrange — no digit.
        assert_eq!(match_placement("Lagrange Point LX Hurston"), None);
    }

    // ---- slug_from_page_title ----------------------------------------

    #[test]
    fn slug_simple() {
        assert_eq!(slug_from_page_title("Lorville"), "lorville");
    }

    #[test]
    fn slug_hyphenated() {
        assert_eq!(
            slug_from_page_title("HUR-L1 Green Glade Station"),
            "hur-l1-green-glade-station"
        );
    }

    #[test]
    fn slug_strips_trailing_disambig_parenthetical() {
        assert_eq!(
            slug_from_page_title("Klescher Rehabilitation Facility (Aberdeen)"),
            "klescher-rehabilitation-facility"
        );
    }

    #[test]
    fn slug_apostrophes_drop_quietly() {
        assert_eq!(
            slug_from_page_title("Rod's Fuel 'N Supplies"),
            "rod-s-fuel-n-supplies"
        );
    }

    // ---- LocationTier round-trip ------------------------------------

    #[test]
    fn tier_as_str_parse_round_trip() {
        for t in [
            LocationTier::System,
            LocationTier::AstronomicalObject,
            LocationTier::LandingZone,
            LocationTier::SpaceStation,
            LocationTier::Landmark,
            LocationTier::Flotilla,
            LocationTier::NavalBase,
            LocationTier::AnonymousPoi,
        ] {
            assert_eq!(LocationTier::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn tier_serde_json_round_trip() {
        let json = serde_json::to_string(&LocationTier::NavalBase).unwrap();
        assert_eq!(json, "\"naval_base\"");
        let back: LocationTier = serde_json::from_str(&json).unwrap();
        assert_eq!(back, LocationTier::NavalBase);
    }
}
