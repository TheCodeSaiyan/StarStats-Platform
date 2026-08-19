//! Engine-log → wiki classification for Star Citizen locations.
//!
//! Companion to [`crate::location_catalog`]. Takes a raw engine
//! string out of a game log (e.g. `OOC_Stanton_3_Lorville`,
//! `LOC_RR_S1_L3`, `Comm_Array_Lagrange_Stanton_L1_HUR-L1`,
//! `LOC_rs_ext_stan-pyro_jp1`) and produces a structured
//! [`LocationClassification`]. The same module runs in two places:
//!
//! * **Tray** (`crates/starstats-client/src/gamelog.rs`): classifies
//!   every parsed event before persisting to the local DB. The
//!   catalogue is the cached snapshot fetched from
//!   `/v1/reference/location` (with a bundled bootstrap so the
//!   first launch with no network still classifies). See
//!   `docs/PLAN-LOCATION-TAXONOMY-V2.md` Phase 4.
//! * **Server** (`crates/starstats-server/src/ingest.rs`):
//!   classifies on `/v1/ingest` so the journey-page rollups can
//!   filter by `location_tier` / `location_subtype` without
//!   round-tripping the full classifier into a query plan.
//!
//! Resolution order (first hit wins):
//!
//!   1. **Synthetic** — engine patterns the wiki doesn't model
//!      (jump points, comm arrays, crash sites, caves, bunkers), plus
//!      *noise* patterns (procedural mining/cluster nodes, dynamic
//!      mission/nav markers) which get an honest generic label and a
//!      suppressible subtype instead of being title-cased into fake
//!      proper-noun places. All map to a synthetic `AnonymousPoi` tier
//!      with a derived subtype.
//!   2. **Catalog (exact)** — `LocationCatalog::lookup_by_token`
//!      against every token in the stripped raw string. The strongest
//!      binding because it pulls real wiki taxonomy.
//!   3. **Catalog (fuzzy)** — `LocationCatalog::fuzzy_match`: idf-
//!      weighted distinctive-token overlap, guarded by a rarity floor
//!      and system consistency. Recovers real wiki rows the engine
//!      names differently (`Stanton4a_RayariHydro_Kaltag` →
//!      `Rayari Kaltag Research Outpost`). Runs before the heuristic
//!      so a real row beats a bare-system guess.
//!   4. **System fallback** — engine string contains a known
//!      system token (`Stanton`/`Pyro`/`Nyx`/…) but no catalogue
//!      hit. Tier left as `AnonymousPoi`; system populated.
//!   5. **Body short-code fallback** — engine emits Lagrange
//!      prefixes like `HUR_L1` or affiliation short codes like
//!      `HDMS_*` / `Shubin_*`. Mapped to the parent system + a
//!      synthetic body name.
//!   6. **Last-resort title-case** — none of the above matched.
//!      Display name is the title-cased raw; tier `AnonymousPoi`.

use std::collections::HashMap;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::location_catalog::{LocationCatalog, LocationCatalogEntry};
use crate::location_taxonomy::{LocationTier, Placement};

/// Output of [`classify`]. Always populated — even on a complete
/// catalog miss, every field has a defined value so downstream
/// consumers don't need a "did the classifier run?" check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationClassification {
    /// Player-friendly name. Pulled from the catalog when matched;
    /// title-cased from the raw on fallback.
    pub display_name: String,
    /// Catalog slug, if matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Tier — always populated (`AnonymousPoi` on miss).
    pub tier: LocationTier,
    /// Sub-bucket from the catalog's `taxonomy.subtype` or the
    /// synthetic matcher's choice (`comm_array`, `jump_point`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    /// Canonical system display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Parent body, when the entity is on or orbiting one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_body: Option<String>,
    /// Spatial relation tag from `starcitizen.tools` enrichment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Placement>,
    /// Engine join token (`tag.name`) when the matched row carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_tag: Option<String>,
    /// Original raw engine string, verbatim. Kept so consumers can
    /// surface it on hover / in detail views for debugging.
    pub raw: String,
    /// Corporate operator (Hurston Dynamics, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    /// In-fiction faction (Nine Tails, XenoThreat, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faction: Option<String>,
    /// Where the binding came from. Useful for telemetry on
    /// catalog-coverage quality.
    pub source: ClassificationSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationSource {
    /// Matched against `LocationCatalog` by an exact key (engine tag,
    /// slug, or normalized name) or a maintained engine-key alias.
    Catalog,
    /// Matched `LocationCatalog` via the distinctive-token fuzzy
    /// fallback. A real wiki binding, but lower-confidence than an
    /// exact key hit — kept distinct for coverage-quality telemetry.
    Fuzzy,
    /// Matched a `SYNTHETIC_MATCHER` (engine-only pattern with no wiki entry).
    Synthetic,
    /// Matched a system / body short-code dictionary.
    Heuristic,
    /// No match — display name is the title-cased raw.
    Fallback,
}

/// Slim wire projection of a [`LocationClassification`] — only the
/// subset the event views need to render a resolved-location link.
///
/// Stamped onto [`crate::wire::EventEnvelope::resolved_location`] by the
/// tray's sync batcher so the tray's recent-events view and the web
/// event views show *identical* resolution: the classifier runs once,
/// on the tray, and the result rides the envelope to the server (which
/// persists it byte-for-byte) and on to the web. The web never
/// re-derives it.
///
/// `slug`/`system` are `skip_serializing_if` so the wire form stays
/// lean and a payload from a pre-resolution client (no key) still
/// deserialises via `#[serde(default)]`. `display_name`, `tier`, and
/// `source` are always present — the classifier guarantees them even on
/// a complete miss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLocation {
    /// Player-friendly name (catalog display, or title-cased raw on a
    /// fallback).
    pub display_name: String,
    /// Catalog slug — present only on a confident catalog/fuzzy hit.
    /// Drives the `/kb/location/{slug}` link; absent → render as plain
    /// text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Canonical system display, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Tier — always populated (`AnonymousPoi` on a miss).
    pub tier: LocationTier,
    /// Where the binding came from. Lets the web treat a `Fuzzy`/
    /// `Catalog` hit differently from a bare `Fallback`/`Heuristic` one.
    pub source: ClassificationSource,
}

impl From<LocationClassification> for ResolvedLocation {
    /// Project the full classification down to the wire subset. Drops
    /// the debugging-only fields (`raw`, `subtype`, `parent_body`,
    /// `placement`, `engine_tag`, `operator`, `faction`) the event
    /// views don't render.
    fn from(c: LocationClassification) -> Self {
        Self {
            display_name: c.display_name,
            slug: c.slug,
            system: c.system,
            tier: c.tier,
            source: c.source,
        }
    }
}

/// Classify an engine string. Pure function — no I/O. See module
/// docs for the resolution order.
pub fn classify(raw: &str, catalog: &LocationCatalog) -> LocationClassification {
    let parts = strip_and_split(raw);
    if parts.is_empty() {
        return fallback(raw, raw);
    }

    // Station engine identifiers do not carry the catalog's slug/name and
    // the upstream rows currently have no engine_tag. Resolve the proven
    // aliases before the generic RR matcher consumes them as anonymous rest
    // stops. A stale/offline catalog simply misses and preserves the generic
    // synthetic fallback below.
    if let Some(c) = match_station_catalog_alias(&parts, raw, catalog) {
        return c;
    }

    // 1. Synthetic patterns. Engine-only constructs that don't
    //    belong in the wiki — match these first so a token like
    //    `jp1` doesn't accidentally hit a catalog row.
    if let Some(c) = SYNTHETIC_MATCHERS.iter().find_map(|m| m(&parts, raw)) {
        return c;
    }

    // 2. Catalog lookup. Try the joined form first (catches
    //    `Stanton1b`-style engine tags), then each segment.
    let joined = parts.join("");
    if let Some(hit) = catalog.lookup_by_token(&joined) {
        return from_catalog(hit, raw, ClassificationSource::Catalog);
    }
    // Exact per-token, but skip bare *system* tokens on this pass. The
    // system itself is a catalogued location (slug `stanton`), and the
    // engine emits the system token FIRST (`OOC_Stanton_2b_Daymar`),
    // so matching it here would shadow the specific body/place that
    // follows — collapsing every planet/moon to its system name.
    // Deferred to a second pass below so a *bare* system identifier
    // still resolves.
    for token in &parts {
        if KNOWN_SYSTEMS.contains_key(token.to_ascii_lowercase().as_str()) {
            continue;
        }
        if let Some(hit) = catalog.lookup_by_token(token) {
            return from_catalog(hit, raw, ClassificationSource::Catalog);
        }
    }

    // 3. Distinctive-token fuzzy match. Runs BEFORE the system
    //    heuristic: a real wiki row (`Rayari Kaltag Research Outpost`)
    //    beats a bare-system guess (`Stanton`). System hint, parsed
    //    from the same parts, prevents cross-system false positives.
    let hint = system_hint(&parts);
    if let Some(hit) = catalog.fuzzy_match(&parts, hint) {
        return from_catalog(hit, raw, ClassificationSource::Fuzzy);
    }

    // 4. Deferred system-token exact match — a bare system identifier
    //    (just `Stanton`) still resolves to the system row with its
    //    full taxonomy now that specific tokens have had priority.
    for token in &parts {
        if let Some(hit) = catalog.lookup_by_token(token) {
            return from_catalog(hit, raw, ClassificationSource::Catalog);
        }
    }

    // 5. System / body short-code heuristics.
    if let Some(c) = system_or_body_heuristic(&parts, raw) {
        return c;
    }

    // 6. Last-resort title-case.
    fallback(&title_case_segments(&parts), raw)
}

/// Best-effort system parse from already-stripped parts, reusing the
/// same dictionaries the heuristic tier uses. Feeds the fuzzy matcher's
/// system-consistency guard.
fn system_hint(parts: &[String]) -> Option<&'static str> {
    for p in parts {
        let key = p.to_ascii_lowercase();
        if let Some(meta) = KNOWN_BODY_SHORT_CODES.get(key.as_str()) {
            return Some(meta.system);
        }
        if let Some(sys) = KNOWN_SYSTEMS.get(key.as_str()) {
            return Some(sys);
        }
    }
    None
}

fn from_catalog(
    hit: &LocationCatalogEntry,
    raw: &str,
    source: ClassificationSource,
) -> LocationClassification {
    LocationClassification {
        display_name: hit.display_name.clone(),
        slug: Some(hit.slug.clone()),
        tier: hit
            .taxonomy
            .tier
            .or_else(|| classification_to_tier(hit.classification.as_deref()))
            .unwrap_or(LocationTier::AnonymousPoi),
        subtype: hit.taxonomy.subtype.clone(),
        system: hit.system.clone(),
        parent_body: hit.parent_body.clone(),
        placement: hit.taxonomy.placement.clone(),
        engine_tag: hit.engine_tag.clone(),
        operator: hit.taxonomy.operator.clone(),
        faction: hit.taxonomy.faction.clone(),
        raw: raw.to_string(),
        source,
    }
}

fn fallback(display: &str, raw: &str) -> LocationClassification {
    LocationClassification {
        display_name: display.to_string(),
        slug: None,
        tier: LocationTier::AnonymousPoi,
        subtype: None,
        system: None,
        parent_body: None,
        placement: None,
        engine_tag: None,
        operator: None,
        faction: None,
        raw: raw.to_string(),
        source: ClassificationSource::Fallback,
    }
}

/// Map api.star-citizen.wiki's coarse `type.classification` strings
/// to our 8-tier model. The wiki uses inconsistent casing
/// (`"Space Station"` vs `"Space station"`); we normalize at lookup.
fn classification_to_tier(c: Option<&str>) -> Option<LocationTier> {
    Some(match c?.to_lowercase().as_str() {
        "solar system" | "system" => LocationTier::System,
        "star" | "planet" | "moon" | "planetoid" | "asteroid belt" | "nebula" => {
            LocationTier::AstronomicalObject
        }
        "landing zone" | "city" => LocationTier::LandingZone,
        "space station" | "station" | "rest stop" | "outpost" => LocationTier::SpaceStation,
        "settlement" => LocationTier::Landmark,
        _ => return None,
    })
}

// ---- strip_and_split -------------------------------------------------

/// Drop engine prefixes / suffixes / boilerplate segments, then
/// split on `_` and any joined `<System><index>` tokens.
///
/// Examples:
///   * `"[PROC]OOC_Stanton_3_Lorville"` → `["Stanton", "3", "Lorville"]`
///   * `"Stanton2_Orison_LOC"` → `["Stanton", "2", "Orison"]`
///   * `"LOC_RR_S1_L3"` → `["RR", "S1", "L3"]`
fn strip_and_split(raw: &str) -> Vec<String> {
    // Drop leading bracketed runtime tag, e.g. `[PROC]` / `[AI_]`.
    let trimmed = raw.trim();
    let trimmed = if trimmed.starts_with('[') {
        if let Some(close) = trimmed.find(']') {
            &trimmed[close + 1..]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    let mut out = Vec::new();
    for seg in trimmed.split('_') {
        let seg = seg.trim();
        if seg.is_empty() || SKIP_SEGMENTS.contains(&seg.to_ascii_uppercase().as_str()) {
            continue;
        }
        for token in split_system_index(seg) {
            out.push(token);
        }
    }
    out
}

/// Engine boilerplate that carries no taxonomic meaning. Compared
/// case-insensitively in upper-case form. Kept short — over-zealous
/// filtering would drop legitimate tokens.
const SKIP_SEGMENTS: &[&str] = &[
    "OOC",  // Object Container
    "LOC",  // Location
    "PROC", // Procedural
    "PAD",  // landing pad
    "NPC",
    "AI",
    "PU",          // Persistent Universe
    "LANDINGAREA", // engine emits `LandingArea_*`
    "OBJECTCONTAINER",
    "NAVPOINT",
    "MISSION",
];

/// Split a `Stanton2` / `Pyro4a` token into `["Stanton", "2"]` /
/// `["Pyro", "4a"]`. Returns `[input]` when no system prefix matches.
fn split_system_index(seg: &str) -> Vec<String> {
    let lower = seg.to_ascii_lowercase();
    for sys in KNOWN_SYSTEMS.keys() {
        if let Some(tail) = lower.strip_prefix(*sys) {
            if !tail.is_empty() && tail.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                let (head, _) = seg.split_at(sys.len());
                let tail_raw = &seg[sys.len()..];
                return vec![head.to_string(), tail_raw.to_string()];
            }
        }
    }
    vec![seg.to_string()]
}

// ---- synthetic matchers ---------------------------------------------

type SyntheticMatcher = fn(parts: &[String], raw: &str) -> Option<LocationClassification>;

/// Bind station-specific engine keys to canonical catalog rows.
///
/// CIG uses two equivalent identifier families for Stanton stations:
///
/// * inventory/death zones: `RR_MIC_LEO`, `RR_HUR_L2`
/// * quantum/object-container targets: `rs_ext_cru-leo1`,
///   `LOC_RR_S2_L1`
///
/// `S1`..`S4` are Stanton planetary sectors (HUR/CRU/ARC/MIC), not
/// star-system numbers. The old generic matcher treated `S2` as Pyro,
/// which both lost the station name and assigned the wrong system.
fn match_station_catalog_alias(
    parts: &[String],
    raw: &str,
    catalog: &LocationCatalog,
) -> Option<LocationClassification> {
    let slug = station_alias_parts(parts)
        .and_then(|(sector, slot)| station_catalog_slug(sector, slot))
        .or_else(|| split_pyro_station_slug(parts))?;
    let hit = catalog.lookup_by_token(slug)?;
    let mut classification = from_catalog(hit, raw, ClassificationSource::Catalog);

    // The upstream station rows are currently classified only as
    // `Manmade` and often lack taxonomy enrichment. The alias shape is
    // station-specific, so it is safe to supply the precise taxonomy here.
    classification.tier = LocationTier::SpaceStation;
    classification.subtype = Some("rest_stop".to_string());
    Some(classification)
}

fn split_pyro_station_slug(parts: &[String]) -> Option<&'static str> {
    if parts.len() != 5
        || !parts[0].eq_ignore_ascii_case("rs")
        || !matches!(
            parts[1].to_ascii_lowercase().as_str(),
            "ext" | "entry" | "comm"
        )
        || !parts[2].eq_ignore_ascii_case("pyro")
    {
        return None;
    }

    let sector = pyro_sector_code(&parts[3])?;
    station_catalog_slug(sector, &parts[4])
}

fn pyro_sector_code(index: &str) -> Option<&'static str> {
    Some(match index {
        "1" => "P1",
        "2" => "P2",
        "3" => "P3",
        "4" => "P4",
        "5" => "P5",
        "6" => "P6",
        _ => return None,
    })
}

fn station_catalog_slug(sector: &str, slot: &str) -> Option<&'static str> {
    stanton_station_slug(sector, slot).or_else(|| {
        let sector = sector.to_ascii_uppercase();
        let slot = slot.to_ascii_uppercase();
        match (sector.as_str(), slot.as_str()) {
            // The current catalog has exactly one manmade station around
            // each body, so these LEO bindings are unambiguous.
            ("P3", "LEO" | "LEO1") => Some("orbituary"),
            ("P6", "LEO" | "LEO1") => Some("ruin-station"),
            _ => None,
        }
    })
}

fn station_alias_parts(parts: &[String]) -> Option<(&str, &str)> {
    if parts.len() == 3 && parts[0].eq_ignore_ascii_case("rr") {
        return Some((&parts[1], &parts[2]));
    }

    if parts.len() == 3
        && parts[0].eq_ignore_ascii_case("rs")
        && matches!(
            parts[1].to_ascii_lowercase().as_str(),
            "ext" | "entry" | "comm"
        )
    {
        let (sector, slot) = parts[2].split_once('-')?;
        return Some((sector, slot));
    }

    None
}

fn stanton_station_slug(sector: &str, slot: &str) -> Option<&'static str> {
    let sector = match sector.to_ascii_uppercase().as_str() {
        "S1" | "HUR" => "HUR",
        "S2" | "CRU" => "CRU",
        "S3" | "ARC" => "ARC",
        "S4" | "MIC" => "MIC",
        _ => return None,
    };
    let slot = match slot.to_ascii_uppercase().as_str() {
        "LEO" | "LEO1" => "LEO",
        "L1" => "L1",
        "L2" => "L2",
        "L3" => "L3",
        "L4" => "L4",
        "L5" => "L5",
        _ => return None,
    };

    Some(match (sector, slot) {
        ("HUR", "LEO") => "everus-harbor",
        ("CRU", "LEO") => "seraphim-station",
        ("ARC", "LEO") => "baijini-point",
        ("MIC", "LEO") => "port-tressler",
        ("HUR", "L1") => "hur-l1-green-glade-station",
        ("HUR", "L2") => "hur-l2-faithful-dream-station",
        ("HUR", "L3") => "hur-l3-thundering-express-station",
        ("HUR", "L4") => "hur-l4-melodic-fields-station",
        ("HUR", "L5") => "hur-l5-high-course-station",
        ("CRU", "L1") => "cru-l1-ambitious-dream-station",
        ("CRU", "L4") => "cru-l4-shallow-fields-station",
        ("CRU", "L5") => "cru-l5-beautiful-glen-station",
        ("ARC", "L1") => "arc-l1-wide-forest-station",
        ("ARC", "L2") => "arc-l2-lively-pathway-station",
        ("ARC", "L3") => "arc-l3-modern-express-station",
        ("ARC", "L4") => "arc-l4-faint-glen-station",
        ("ARC", "L5") => "arc-l5-yellow-core-station",
        ("MIC", "L1") => "mic-l1-shallow-frontier-station",
        ("MIC", "L2") => "mic-l2-long-forest-station",
        ("MIC", "L3") => "mic-l3-endless-odyssey-station",
        ("MIC", "L4") => "mic-l4-red-crossroads-station",
        ("MIC", "L5") => "mic-l5-modern-icarus-station",
        _ => return None,
    })
}

static SYNTHETIC_MATCHERS: &[SyntheticMatcher] = &[
    match_gateway,
    match_jump_point,
    match_nyx_qv_extraction_station,
    match_comm_array,
    match_crash_site,
    match_cave,
    match_bunker,
    match_derelict,
    // Three engine-prefix patterns the wiki doesn't model directly:
    // RR_… (Rest Stop / R&R loadouts), OM_… (Orbital Markers), and
    // a bare `RestStop` token (UI-level "nearest rest stop"
    // selection). All produce a classified result with a more
    // useful subtype than the heuristic fallback would.
    match_rest_stop_engine,
    match_rest_stop_generic,
    match_orbital_marker,
    // Noise patterns — engine-only dynamic / procedural identifiers
    // that have no catalogued wiki page. Classified with an honest
    // generic label + a suppressible subtype so they stop being
    // title-cased into fake proper-noun "places" (e.g.
    // `ab_mine_stanton2_med_010` → "Asteroid mining node", not
    // "Ab Mine Stanton 2 Med 010").
    match_dynamic_marker,
    match_procedural_node,
];

/// Nyx's procedural extraction-station family. The engine keys carry a
/// numeric instance (`Nyx_TSG_QVExtractionStation_035`), but the catalog
/// currently exposes dozens of indistinguishable QV Breaker rows without
/// engine tags. Surface the correct family and hierarchy while withholding
/// a slug rather than linking to an arbitrary duplicate.
fn match_nyx_qv_extraction_station(parts: &[String], raw: &str) -> Option<LocationClassification> {
    let is_nyx = parts.iter().any(|part| part.eq_ignore_ascii_case("nyx"));
    let is_qv_extraction = parts
        .iter()
        .any(|part| part.eq_ignore_ascii_case("qvextractionstation"));
    if !is_nyx || !is_qv_extraction {
        return None;
    }

    Some(LocationClassification {
        display_name: "QV Breaker Station".to_string(),
        slug: None,
        tier: LocationTier::SpaceStation,
        subtype: Some("breaker_station".to_string()),
        system: Some("Nyx".to_string()),
        parent_body: Some("Nyx".to_string()),
        placement: None,
        engine_tag: None,
        operator: None,
        faction: None,
        raw: raw.to_string(),
        source: ClassificationSource::Synthetic,
    })
}

/// Gateway detection. The engine emits `JP_<SystemA>_<SystemB>` for
/// the space stations that serve as jump-gate terminals; they are named
/// after the **destination** system (the second token). Examples:
///
///   * `JP_Stanton_Pyro`  → "Pyro Gateway"  (physically in Stanton)
///   * `JP_Pyro_Stanton`  → "Stanton Gateway" (physically in Pyro)
///
/// Gate: first token is exactly `jp` (case-insensitive, **no** trailing
/// digit — `jp1` belongs to [`match_jump_point`], not here), followed
/// by exactly two tokens that both resolve in [`KNOWN_SYSTEMS`].
///
/// Must be registered **before** `match_jump_point` so that the two-
/// system `JP_<A>_<B>` form is consumed here while the digit-suffixed
/// `rs_*_jpN` / `<sys>_jpN` forms fall through untouched.
fn match_gateway(parts: &[String], raw: &str) -> Option<LocationClassification> {
    // Require at least 3 tokens: [jp, sysA, sysB] (any extras are noise).
    if parts.len() < 3 {
        return None;
    }
    // First token must be exactly "jp" — no digits.
    let first = parts[0].to_ascii_lowercase();
    if first != "jp" {
        return None;
    }
    // The two tokens immediately after "jp" must both be known systems.
    let key_a = parts[1].to_ascii_lowercase();
    let key_b = parts[2].to_ascii_lowercase();
    let sys_a = KNOWN_SYSTEMS.get(key_a.as_str())?; // origin system (physical location)
    let sys_b = KNOWN_SYSTEMS.get(key_b.as_str())?; // destination system (namesake)
    let display = format!("{} Gateway", sys_b);
    Some(LocationClassification {
        display_name: display,
        slug: None,
        tier: LocationTier::SpaceStation,
        subtype: Some("gateway".to_string()),
        system: Some(sys_a.to_string()),
        parent_body: None,
        placement: None,
        engine_tag: None,
        operator: None,
        faction: None,
        raw: raw.to_string(),
        source: ClassificationSource::Synthetic,
    })
}

/// Jump-point detection. The shape gating the matcher is simply the
/// presence of a `jp<digit>` token — anywhere in the parts list. The
/// engine emits at least four real-world variants:
///
///   * `LOC_rs_ext_stan-pyro_jp1`     — original hyphenated endpoints
///   * `rs_entry_nyx_pyro_jp1`        — separator-form, verb=`entry`
///   * `rs_comm_nyx_castra_jp1`       — separator-form, verb=`comm`
///   * `pyro_jp1` / `magnus_jp1`      — bare `<system>_jp<N>`
///
/// All four mean "this is a jump point". The `rs_<verb>_<a>_<b>_jpN`
/// shape carries the two endpoints; the bare shape carries only the
/// origin system. Earlier the matcher gated on `rs ext` literally,
/// which dropped every variant except the first — confirmed by
/// auditing a real LIVE log (2026-05-26) that had only `rs_entry`,
/// `rs_comm`, and bare forms. Now any `jp<N>` token triggers the
/// match; endpoint extraction adapts to whichever shape is present.
fn match_jump_point(parts: &[String], raw: &str) -> Option<LocationClassification> {
    // Find the `jp<digit>` token. Required signal — without it we
    // never fire (avoids matching unrelated identifiers).
    let mut jp_index: Option<u32> = None;
    let mut jp_pos: Option<usize> = None;
    for (i, p) in parts.iter().enumerate() {
        let lower = p.to_ascii_lowercase();
        if let Some(num) = lower.strip_prefix("jp") {
            if let Ok(n) = num.parse::<u32>() {
                jp_index = Some(n);
                jp_pos = Some(i);
                break;
            }
        }
    }
    let jp_pos = jp_pos?;

    // Endpoint resolution — three shapes:
    //
    //   shape A: hyphenated pair (`stan-pyro`) as a single token,
    //            following an `rs ext` pair.
    //   shape B: two consecutive tokens immediately preceding the
    //            `jp<N>` (`rs entry nyx pyro jp1` → ("nyx","pyro")).
    //   shape C: a single system token preceding the `jp<N>`
    //            (`magnus_jp1` → just "magnus"; cross-system not
    //            specified, but at least the origin is known).
    let mut endpoints: Option<(String, String)> = None;
    let mut origin: Option<String> = None;
    // Shape A — look for `<a>-<b>` anywhere left of the jp token.
    for p in &parts[..jp_pos] {
        if let Some((a, b)) = p.split_once('-') {
            if !a.is_empty() && !b.is_empty() {
                endpoints = Some((title_case_word(a), title_case_word(b)));
                break;
            }
        }
    }
    // Shape B — the two tokens immediately before jp, when neither
    // is a known engine keyword (`rs`/`ext`/`entry`/`comm`).
    if endpoints.is_none() && jp_pos >= 2 {
        let a = &parts[jp_pos - 2];
        let b = &parts[jp_pos - 1];
        let kw = |t: &String| {
            let l = t.to_ascii_lowercase();
            matches!(l.as_str(), "rs" | "ext" | "entry" | "comm")
        };
        if !kw(a) && !kw(b) {
            endpoints = Some((title_case_word(a), title_case_word(b)));
        }
    }
    // Shape C — single token before jp, used as origin only.
    if endpoints.is_none() && jp_pos >= 1 {
        let a = &parts[jp_pos - 1];
        let l = a.to_ascii_lowercase();
        if !matches!(l.as_str(), "rs" | "ext" | "entry" | "comm") {
            origin = Some(title_case_word(a));
        }
    }

    let display = match (&endpoints, &origin, jp_index) {
        (Some((a, b)), _, Some(n)) => format!("{a} ↔ {b} jump point #{n}"),
        (Some((a, b)), _, None) => format!("{a} ↔ {b} jump point"),
        (None, Some(o), Some(n)) => format!("{o} jump point #{n}"),
        (None, Some(o), None) => format!("{o} jump point"),
        _ => "Jump point".to_string(),
    };
    let system = endpoints
        .as_ref()
        .map(|(a, _)| a.clone())
        .or_else(|| origin.clone());
    Some(synthetic(display, "jump_point", system, raw))
}

/// Comm arrays: tokens contain `comm_array` or `commarray`. The
/// engine emits `Comm_Array_Lagrange_Stanton_L1_HUR-L1` and
/// similar. We expose tier=AnonymousPoi + subtype=comm_array.
fn match_comm_array(parts: &[String], raw: &str) -> Option<LocationClassification> {
    let lower = raw.to_ascii_lowercase();
    if !lower.contains("comm_array") && !lower.contains("commarray") {
        return None;
    }
    // Best-effort: find the trailing `<short>-Lx` token (HUR-L1,
    // ARC-L2, …) and route the system off the short code.
    let system = parts.iter().find_map(|p| {
        let first = p.split('-').next()?;
        body_short_code_system(first)
    });
    Some(synthetic(
        format!("Comm array ({})", parts.last().cloned().unwrap_or_default()),
        "comm_array",
        system.map(str::to_string),
        raw,
    ))
}

fn match_crash_site(parts: &[String], raw: &str) -> Option<LocationClassification> {
    let lower = raw.to_ascii_lowercase();
    if !lower.contains("crash_site") && !lower.contains("crashsite") {
        return None;
    }
    Some(synthetic(
        title_case_segments(parts),
        "crash_site",
        None,
        raw,
    ))
}

fn match_cave(parts: &[String], raw: &str) -> Option<LocationClassification> {
    let lower = raw.to_ascii_lowercase();
    let any_cave = parts
        .iter()
        .any(|p| p.eq_ignore_ascii_case("cave") || p.eq_ignore_ascii_case("caverns"));
    if !any_cave && !lower.contains("cave") {
        return None;
    }
    Some(synthetic(title_case_segments(parts), "cave", None, raw))
}

fn match_bunker(parts: &[String], raw: &str) -> Option<LocationClassification> {
    let any_bunker = parts.iter().any(|p| p.eq_ignore_ascii_case("bunker"));
    if !any_bunker {
        return None;
    }
    Some(synthetic(title_case_segments(parts), "bunker", None, raw))
}

fn match_derelict(parts: &[String], raw: &str) -> Option<LocationClassification> {
    let lower = raw.to_ascii_lowercase();
    if !lower.contains("derelict") && !lower.contains("salvage_yard") {
        return None;
    }
    Some(synthetic(
        title_case_segments(parts),
        "derelict_ship",
        None,
        raw,
    ))
}

/// Engine-side Rest Stop loadout prefix. CIG ships engine identifiers
/// like `LOC_RR_S1_L3` (R&R loadout at Hurston's L3 point) and
/// `rs_ext_pyro3_l1` for rest stops whose wiki entries are catalogued
/// under their human names. When the maintained catalog alias hits,
/// it wins before this matcher. This fallback still gives unknown or
/// not-yet-catalogued keys a `SpaceStation / rest_stop` classification.
///
/// `parts` ordering: either an `RR` token followed by a Stanton sector
/// or body code, or `rs` + `ext|entry|comm` followed by station context.
/// We do NOT invent a specific station name here — that's the catalog
/// alias's job. Display name is a best-effort title-case of the tail.
fn match_rest_stop_engine(parts: &[String], raw: &str) -> Option<LocationClassification> {
    // Exact tokens reject substring matches (`ROARRR` etc.). Jump-point
    // variants with the same `rs_*` prefix have already been consumed by
    // `match_jump_point`, which runs earlier in SYNTHETIC_MATCHERS.
    let tail = if let Some(rr_idx) = parts.iter().position(|p| p.eq_ignore_ascii_case("rr")) {
        &parts[rr_idx + 1..]
    } else if parts.len() >= 3
        && parts[0].eq_ignore_ascii_case("rs")
        && matches!(
            parts[1].to_ascii_lowercase().as_str(),
            "ext" | "entry" | "comm"
        )
    {
        &parts[2..]
    } else {
        return None;
    };

    // Walk the tail to resolve as much context as we can. Three
    // independent signals: a direct system token, an `S1`..`S4`
    // Stanton sector, or a body short code (`MIC`, `HUR`, …) which
    // carries the system implicitly. Real-world example:
    // `RR_MIC_LEO` — no system token at all, but `MIC` short-codes
    // microTech, so system = Stanton.
    let mut system: Option<String> = None;
    let mut parent_body: Option<String> = None;
    for (index, p) in tail.iter().enumerate() {
        let lower = p.to_ascii_lowercase();
        // `rs_ext_cru-leo1` keeps the sector and slot in one hyphenated
        // token. Only the sector prefix carries hierarchy context.
        let context = lower
            .split_once('-')
            .map_or(lower.as_str(), |(head, _)| head);
        if system.is_none() {
            if let Some(canon) = KNOWN_SYSTEMS.get(context) {
                system = Some(canon.to_string());
                if *canon == "Pyro" {
                    parent_body = tail
                        .get(index + 1)
                        .and_then(|next| pyro_sector_code(next))
                        .and_then(|code| {
                            KNOWN_BODY_SHORT_CODES.get(code.to_ascii_lowercase().as_str())
                        })
                        .map(|meta| meta.body_display.to_string());
                }
                continue;
            }
            match context {
                "s1" => {
                    system = Some("Stanton".to_string());
                    parent_body = Some("Hurston".to_string());
                    continue;
                }
                "s2" => {
                    system = Some("Stanton".to_string());
                    parent_body = Some("Crusader".to_string());
                    continue;
                }
                "s3" => {
                    system = Some("Stanton".to_string());
                    parent_body = Some("ArcCorp".to_string());
                    continue;
                }
                "s4" => {
                    system = Some("Stanton".to_string());
                    parent_body = Some("microTech".to_string());
                    continue;
                }
                _ => {}
            }
        }
        if let Some(meta) = KNOWN_BODY_SHORT_CODES.get(context) {
            if system.is_none() {
                system = Some(meta.system.to_string());
            }
            if parent_body.is_none() {
                parent_body = Some(meta.body_display.to_string());
            }
        }
    }

    let display = if tail.is_empty() {
        "Rest Stop".to_string()
    } else {
        format!("Rest Stop ({})", title_case_segments(tail))
    };

    Some(LocationClassification {
        display_name: display,
        slug: None,
        tier: LocationTier::SpaceStation,
        subtype: Some("rest_stop".to_string()),
        system,
        parent_body,
        placement: None,
        engine_tag: None,
        operator: None,
        faction: None,
        raw: raw.to_string(),
        source: ClassificationSource::Synthetic,
    })
}

/// Generic `RestStop` token matcher — catches engine identifiers
/// like `ObjectContainer_RestStop` that the player's quantum-target
/// UI emits when selecting "nearest rest stop" rather than a
/// specific station. No system / body context available; we just
/// tag the subtype so the journey rollup can bucket these
/// distinctly from other space-stations.
fn match_rest_stop_generic(parts: &[String], raw: &str) -> Option<LocationClassification> {
    let has_token = parts
        .iter()
        .any(|p| p.eq_ignore_ascii_case("reststop") || p.eq_ignore_ascii_case("rest_stop"));
    if !has_token {
        return None;
    }
    Some(LocationClassification {
        display_name: "Rest Stop".to_string(),
        slug: None,
        tier: LocationTier::SpaceStation,
        subtype: Some("rest_stop".to_string()),
        system: None,
        parent_body: None,
        placement: None,
        engine_tag: None,
        operator: None,
        faction: None,
        raw: raw.to_string(),
        source: ClassificationSource::Synthetic,
    })
}

/// Orbital marker engine prefix. CIG places 6 navigational markers
/// (OM-1..OM-6) around each celestial body; the engine emits these
/// in strings like `OM_Hurston_1`, `OM-1_Daymar`, or
/// `LOC_OM_Crusader_4`. They have no wiki entries (they're pure
/// navigation reference points), so a SYNTHETIC classification is
/// the right home. Produces tier=AnonymousPoi + subtype=orbital_marker
/// with the parent_body resolved off any body short-code token.
fn match_orbital_marker(parts: &[String], raw: &str) -> Option<LocationClassification> {
    // Engines emit OM in three shapes:
    //   1. A standalone `OM` token followed by a bare digit token
    //      (`OM_Hurston_3` → parts `["OM", "Hurston", "3"]`).
    //   2. A glued `OM-3` / `OM_3` / `OM3` token.
    //   3. A standalone `OM` with no index at all (rare; we still
    //      classify as orbital_marker with no index).
    // Detection is exact-token to avoid catching unrelated words
    // containing "om" (e.g. "Lorville" has "om" as a substring).
    let mut om_index: Option<u32> = None;
    let mut saw_om = false;
    for p in parts {
        let lower = p.to_ascii_lowercase();
        if lower == "om" {
            saw_om = true;
            continue;
        }
        // OM-1, OM_1, OM1, OM-12 — strip the prefix and parse the rest.
        if let Some(rest) = lower
            .strip_prefix("om-")
            .or_else(|| lower.strip_prefix("om_"))
            .or_else(|| {
                if lower.starts_with("om")
                    && lower.len() > 2
                    && lower.as_bytes()[2].is_ascii_digit()
                {
                    Some(&lower[2..])
                } else {
                    None
                }
            })
        {
            if let Ok(n) = rest.parse::<u32>() {
                saw_om = true;
                om_index = Some(n);
            }
        }
    }
    if !saw_om {
        return None;
    }
    // If a standalone OM token was seen but no glued index, hunt
    // through the remaining parts for a small bare digit token
    // (1..99 covers any plausible OM index — CIG ships 6 per body
    // today, leaving headroom). We accept only the first hit so
    // unrelated trailing numerics don't get adopted.
    if om_index.is_none() {
        om_index = parts.iter().find_map(|p| {
            let n: u32 = p.parse().ok()?;
            if (1..=99).contains(&n) {
                Some(n)
            } else {
                None
            }
        });
    }

    // Look for a body short code in the remaining segments — same
    // dictionary the heuristic fallback uses, so the OM resolves to
    // a real parent body when CIG includes one.
    let body_meta = parts
        .iter()
        .find_map(|p| KNOWN_BODY_SHORT_CODES.get(p.to_ascii_lowercase().as_str()));

    let display = match (body_meta, om_index) {
        (Some(meta), Some(n)) => format!("{} OM-{}", meta.body_display, n),
        (Some(meta), None) => format!("{} orbital marker", meta.body_display),
        (None, Some(n)) => format!("OM-{}", n),
        (None, None) => "Orbital marker".to_string(),
    };

    Some(LocationClassification {
        display_name: display,
        slug: None,
        tier: LocationTier::AnonymousPoi,
        subtype: Some("orbital_marker".to_string()),
        system: body_meta.map(|m| m.system.to_string()),
        parent_body: body_meta.map(|m| m.body_display.to_string()),
        placement: body_meta.map(|m| Placement::OrbitsBody {
            body: m.body_display.to_string(),
        }),
        engine_tag: None,
        operator: None,
        faction: None,
        raw: raw.to_string(),
        source: ClassificationSource::Synthetic,
    })
}

/// Dynamic, per-session engine markers: mission quantum-travel
/// beacons and dynamically-spawned nav points. These carry a runtime
/// entity id (`MISSION_QT_Quantum_Beacon_286174403838`) and never
/// correspond to a fixed wiki location. Labelled generically so the
/// render layer can suppress them via `subtype` instead of surfacing a
/// fabricated place name.
fn match_dynamic_marker(_parts: &[String], raw: &str) -> Option<LocationClassification> {
    let lower = raw.to_ascii_lowercase();
    let (display, subtype) =
        if lower.contains("navpoint_dynamic") || lower.contains("dynamic_navpoint") {
            ("Dynamic nav point", "nav_marker")
        } else if lower.contains("mission_qt") || lower.contains("quantum_beacon") {
            ("Mission marker", "mission_marker")
        } else {
            return None;
        };
    Some(synthetic(display.to_string(), subtype, None, raw))
}

/// Procedural / instanced resource sites: asteroid mining and gas
/// collection nodes, asteroid clusters (`*.socpak` object containers),
/// and static race tracks. Real places players visit, but not
/// catalogued wiki entities — so we give an honest category label and
/// attach the system when the engine string carries one.
fn match_procedural_node(parts: &[String], raw: &str) -> Option<LocationClassification> {
    let lower = raw.to_ascii_lowercase();
    let (display, subtype) = if lower.contains("ab_mine") {
        ("Asteroid mining node", "mining_node")
    } else if lower.contains("ab_collector") {
        ("Gas collection node", "gas_node")
    } else if lower.contains("_cluster_") || lower.ends_with(".socpak") {
        ("Asteroid cluster", "asteroid_cluster")
    } else if lower.contains("racing_static") {
        ("Race track", "race_track")
    } else {
        return None;
    };
    Some(synthetic(
        display.to_string(),
        subtype,
        system_hint(parts).map(str::to_string),
        raw,
    ))
}

fn synthetic(
    display: String,
    subtype: &str,
    system: Option<String>,
    raw: &str,
) -> LocationClassification {
    LocationClassification {
        display_name: display,
        slug: None,
        tier: LocationTier::AnonymousPoi,
        subtype: Some(subtype.to_string()),
        system,
        parent_body: None,
        placement: None,
        engine_tag: None,
        operator: None,
        faction: None,
        raw: raw.to_string(),
        source: ClassificationSource::Synthetic,
    }
}

// ---- system / body short-code heuristic ----------------------------

fn system_or_body_heuristic(parts: &[String], raw: &str) -> Option<LocationClassification> {
    // Try a known-body short code first — those carry both system
    // AND body, so they're strictly more informative than a bare
    // system match.
    for p in parts {
        if let Some(meta) = KNOWN_BODY_SHORT_CODES.get(p.to_ascii_lowercase().as_str()) {
            return Some(LocationClassification {
                display_name: meta.body_display.to_string(),
                slug: None,
                tier: LocationTier::AstronomicalObject,
                subtype: None,
                system: Some(meta.system.to_string()),
                parent_body: None,
                placement: None,
                engine_tag: None,
                operator: None,
                faction: None,
                raw: raw.to_string(),
                source: ClassificationSource::Heuristic,
            });
        }
    }
    // Bare system fallback — at least we know which star we're in.
    for p in parts {
        if let Some(system) = KNOWN_SYSTEMS.get(p.to_ascii_lowercase().as_str()) {
            return Some(LocationClassification {
                display_name: title_case_segments(parts),
                slug: None,
                tier: LocationTier::AnonymousPoi,
                subtype: None,
                system: Some(system.to_string()),
                parent_body: None,
                placement: None,
                engine_tag: None,
                operator: None,
                faction: None,
                raw: raw.to_string(),
                source: ClassificationSource::Heuristic,
            });
        }
    }
    None
}

// ---- static lookup tables -----------------------------------------

static KNOWN_SYSTEMS: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    // Keys lowercase, values canonical display. Short list — a miss
    // falls through to AnonymousPoi rather than mis-attributing.
    let mut m = HashMap::with_capacity(6);
    m.insert("stanton", "Stanton");
    m.insert("pyro", "Pyro");
    m.insert("nyx", "Nyx");
    m.insert("castra", "Castra");
    m.insert("terra", "Terra");
    m.insert("sol", "Sol");
    m
});

struct BodyShortCodeMeta {
    system: &'static str,
    body_display: &'static str,
}

static KNOWN_BODY_SHORT_CODES: Lazy<HashMap<&'static str, BodyShortCodeMeta>> = Lazy::new(|| {
    // Stanton Lagrange-point prefixes (`HUR_L1_…`, `CRU_L4_…`).
    // Each names the planet they orbit. Tier 0 catalog wins when
    // a specific station is catalogued; this is the engine-only
    // pattern that needs a body without the wiki.
    let mut m = HashMap::new();
    let entries: &[(&str, &str, &str)] = &[
        // Lagrange prefixes
        ("hur", "Stanton", "Hurston"),
        ("cru", "Stanton", "Crusader"),
        ("arc", "Stanton", "ArcCorp"),
        ("mic", "Stanton", "microTech"),
        // Affiliation short codes
        ("hurdyn", "Stanton", "Hurston"),
        ("shubin", "Stanton", "Shubin Outposts"),
        ("hdms", "Stanton", "HDMS Outposts"),
        ("rr", "Stanton", "Rest Stops"),
        // System stars
        ("stantonstar", "Stanton", "Stanton"),
        ("pyrostar", "Pyro", "Pyro"),
        ("nyxstar", "Nyx", "Nyx"),
        // Common body short codes (when catalog hasn't been seeded).
        ("hurston", "Stanton", "Hurston"),
        ("crusader", "Stanton", "Crusader"),
        ("arccorp", "Stanton", "ArcCorp"),
        ("microtech", "Stanton", "microTech"),
        ("daymar", "Stanton", "Daymar"),
        ("yela", "Stanton", "Yela"),
        ("cellin", "Stanton", "Cellin"),
        ("aberdeen", "Stanton", "Aberdeen"),
        ("magda", "Stanton", "Magda"),
        ("ita", "Stanton", "Ita"),
        ("arial", "Stanton", "Arial"),
        // Engine retains the legacy `Ariel` spelling (e.g.
        // `OOC_Stanton_1a_Ariel`); CIG renamed the moon to `Arial`
        // in Alpha 3.3.0 to disambiguate from Uranus' moon Ariel,
        // but the engine identifier never updated. Map both to the
        // canonical wiki display.
        ("ariel", "Stanton", "Arial"),
        ("wala", "Stanton", "Wala"),
        ("lyria", "Stanton", "Lyria"),
        ("calliope", "Stanton", "Calliope"),
        ("clio", "Stanton", "Clio"),
        ("euterpe", "Stanton", "Euterpe"),
        ("bloom", "Pyro", "Bloom"),
        ("monox", "Pyro", "Monox"),
        ("terminus", "Pyro", "Terminus"),
        // Pyro rest-stop prefixes (`RR_P3_LEO`, `RR_P5_L4`, ...).
        // These are planet indices, not star-system identifiers.
        ("p1", "Pyro", "Pyro I"),
        ("p2", "Pyro", "Monox"),
        ("p3", "Pyro", "Bloom"),
        ("p4", "Pyro", "Pyro IV"),
        ("p5", "Pyro", "Pyro V"),
        ("p6", "Pyro", "Terminus"),
        ("delamar", "Nyx", "Delamar"),
        // Landing-zone heuristics — strictly these are cities not
        // bodies, but the engine emits identifiers like
        // `Area18_City_objectContainer` that the catalog alone may
        // not have keyed under "Area18". Adding here gives a usable
        // system+parent fallback until the catalog is seeded.
        ("area18", "Stanton", "Area18"),
        ("a18", "Stanton", "Area18"),
        ("lorville", "Stanton", "Lorville"),
        ("newbabbage", "Stanton", "New Babbage"),
        ("orison", "Stanton", "Orison"),
    ];
    for (k, sys, body) in entries {
        m.insert(
            *k,
            BodyShortCodeMeta {
                system: sys,
                body_display: body,
            },
        );
    }
    m
});

fn body_short_code_system(token: &str) -> Option<&'static str> {
    KNOWN_BODY_SHORT_CODES
        .get(token.to_ascii_lowercase().as_str())
        .map(|m| m.system)
}

// ---- formatting helpers --------------------------------------------

fn title_case_segments(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| title_case_word(p))
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_case_word(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut next_upper = true;
    for ch in s.chars() {
        if ch == '-' {
            out.push(ch);
            next_upper = true;
            continue;
        }
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
    use crate::location_catalog::LocationCatalogEntry;
    use crate::location_taxonomy::LocationTaxonomy;

    #[test]
    fn resolved_location_projects_wire_subset_from_classification() {
        let full = LocationClassification {
            display_name: "Rayari Kaltag Research Outpost".into(),
            slug: Some("rayari-kaltag-research-outpost".into()),
            tier: LocationTier::Landmark,
            subtype: Some("research".into()),
            system: Some("Stanton".into()),
            parent_body: Some("Kaltag".into()),
            placement: None,
            engine_tag: None,
            raw: "Stanton4a_RayariHydro_Kaltag".into(),
            operator: Some("Rayari".into()),
            faction: None,
            source: ClassificationSource::Fuzzy,
        };
        let wire: ResolvedLocation = full.into();
        assert_eq!(wire.display_name, "Rayari Kaltag Research Outpost");
        assert_eq!(wire.slug.as_deref(), Some("rayari-kaltag-research-outpost"));
        assert_eq!(wire.system.as_deref(), Some("Stanton"));
        assert_eq!(wire.tier, LocationTier::Landmark);
        assert_eq!(wire.source, ClassificationSource::Fuzzy);
    }

    #[test]
    fn resolved_location_omits_slug_and_system_when_absent_on_wire() {
        let miss = ResolvedLocation {
            display_name: "Some Unknown Place".into(),
            slug: None,
            system: None,
            tier: LocationTier::AnonymousPoi,
            source: ClassificationSource::Fallback,
        };
        let json = serde_json::to_string(&miss).unwrap();
        assert!(!json.contains("\"slug\""));
        assert!(!json.contains("\"system\""));
        // Round-trips, and an absent slug/system deserialises to None.
        let back: ResolvedLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(miss, back);
    }

    fn empty_catalog() -> LocationCatalog {
        LocationCatalog::from_entries(vec![])
    }

    fn catalog_with(entries: Vec<LocationCatalogEntry>) -> LocationCatalog {
        LocationCatalog::from_entries(entries)
    }

    fn aberdeen_entry() -> LocationCatalogEntry {
        LocationCatalogEntry {
            slug: "aberdeen".into(),
            display_name: "Aberdeen".into(),
            class_name: "Aberdeen".into(),
            engine_tag: Some("Stanton1b".into()),
            system: Some("Stanton".into()),
            parent_body: Some("Hurston".into()),
            classification: Some("Moon".into()),
            taxonomy: LocationTaxonomy {
                tier: Some(LocationTier::AstronomicalObject),
                subtype: Some("moon".into()),
                ..LocationTaxonomy::default()
            },
        }
    }

    fn lorville_entry() -> LocationCatalogEntry {
        LocationCatalogEntry {
            slug: "lorville".into(),
            display_name: "Lorville".into(),
            class_name: "Lorville".into(),
            engine_tag: Some("Lorville".into()),
            system: Some("Stanton".into()),
            parent_body: Some("Hurston".into()),
            classification: Some("Settlement".into()),
            taxonomy: LocationTaxonomy {
                tier: Some(LocationTier::LandingZone),
                subtype: Some("city".into()),
                placement: Some(Placement::OnBody {
                    body: "Hurston".into(),
                }),
                operator: Some("Hurston Dynamics".into()),
                ..LocationTaxonomy::default()
            },
        }
    }

    fn station_entry(slug: &str, display_name: &str, parent_body: &str) -> LocationCatalogEntry {
        station_entry_in_system(slug, display_name, "Stanton", parent_body)
    }

    fn station_entry_in_system(
        slug: &str,
        display_name: &str,
        system: &str,
        parent_body: &str,
    ) -> LocationCatalogEntry {
        LocationCatalogEntry {
            slug: slug.into(),
            display_name: display_name.into(),
            class_name: slug.into(),
            engine_tag: None,
            system: Some(system.into()),
            parent_body: Some(parent_body.into()),
            classification: Some("Manmade".into()),
            taxonomy: LocationTaxonomy::default(),
        }
    }

    // ---- catalog hits ----------------------------------------------

    #[test]
    fn catalog_hit_via_joined_engine_tag() {
        // `Stanton1b` is the engine tag for Aberdeen — embedded in
        // longer engine strings like `OOC_Stanton_1b_Aberdeen`.
        let cat = catalog_with(vec![aberdeen_entry()]);
        let c = classify("OOC_Stanton_1b_Aberdeen", &cat);
        assert_eq!(c.source, ClassificationSource::Catalog);
        assert_eq!(c.display_name, "Aberdeen");
        assert_eq!(c.tier, LocationTier::AstronomicalObject);
        assert_eq!(c.subtype.as_deref(), Some("moon"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.parent_body.as_deref(), Some("Hurston"));
    }

    #[test]
    fn catalog_hit_via_slug() {
        let cat = catalog_with(vec![lorville_entry()]);
        let c = classify("OOC_Stanton_3_Lorville", &cat);
        assert_eq!(c.source, ClassificationSource::Catalog);
        assert_eq!(c.display_name, "Lorville");
        assert_eq!(c.tier, LocationTier::LandingZone);
        assert_eq!(c.subtype.as_deref(), Some("city"));
        assert_eq!(c.operator.as_deref(), Some("Hurston Dynamics"));
        assert_eq!(
            c.placement,
            Some(Placement::OnBody {
                body: "Hurston".into()
            })
        );
    }

    #[test]
    fn catalog_hit_propagates_engine_tag_to_classification() {
        let cat = catalog_with(vec![aberdeen_entry()]);
        let c = classify("OOC_Stanton_1b_Aberdeen", &cat);
        assert_eq!(c.engine_tag.as_deref(), Some("Stanton1b"));
    }

    // ---- synthetic matchers ----------------------------------------

    // ---- gateway matcher -------------------------------------------

    #[test]
    fn gateway_jp_stanton_pyro_yields_pyro_gateway_in_stanton() {
        // The live user case: `JP_Stanton_Pyro` is "Pyro Gateway"
        // (named after the destination) physically located in Stanton.
        let c = classify("JP_Stanton_Pyro", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.display_name, "Pyro Gateway");
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.subtype.as_deref(), Some("gateway"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
    }

    #[test]
    fn gateway_jp_pyro_stanton_yields_stanton_gateway_in_pyro() {
        // Reversed direction: `JP_Pyro_Stanton` sits in Pyro and its
        // destination is Stanton → "Stanton Gateway".
        let c = classify("JP_Pyro_Stanton", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.display_name, "Stanton Gateway");
        assert_eq!(c.system.as_deref(), Some("Pyro"));
        assert_eq!(c.subtype.as_deref(), Some("gateway"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
    }

    #[test]
    fn gateway_does_not_fire_on_jump_point_with_digit_suffix() {
        // `LOC_rs_ext_stan-pyro_jp1` still classifies as jump_point,
        // NOT gateway — the trailing digit on `jp1` means match_gateway
        // should not fire (the `jp` token has a suffix).
        let c = classify("LOC_rs_ext_stan-pyro_jp1", &empty_catalog());
        assert_ne!(c.subtype.as_deref(), Some("gateway"));
        assert_eq!(c.subtype.as_deref(), Some("jump_point"));
    }

    #[test]
    fn gateway_does_not_fire_on_single_system_token() {
        // Only one system after jp — not enough for the gateway form.
        let c = classify("JP_Stanton", &empty_catalog());
        assert_ne!(c.subtype.as_deref(), Some("gateway"));
    }

    #[test]
    fn gateway_does_not_fire_on_unknown_system_tokens() {
        // Unknown tokens after jp should not produce a gateway match.
        let c = classify("JP_Foobar_Baz", &empty_catalog());
        assert_ne!(c.subtype.as_deref(), Some("gateway"));
    }

    // ---- jump-point synthetic matchers (existing) ------------------

    #[test]
    fn jump_point_synthetic_matches_stan_to_pyro() {
        let c = classify("LOC_rs_ext_stan-pyro_jp1", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("jump_point"));
        assert!(c.display_name.contains("Stan"));
        assert!(c.display_name.contains("Pyro"));
        assert_eq!(c.tier, LocationTier::AnonymousPoi);
    }

    #[test]
    fn comm_array_synthetic_picks_subtype_and_attempts_system() {
        let c = classify("Comm_Array_Lagrange_Stanton_L1_HUR-L1", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("comm_array"));
        // System derives off the `HUR-L1` body short code.
        assert_eq!(c.system.as_deref(), Some("Stanton"));
    }

    #[test]
    fn crash_site_synthetic_recognised() {
        let c = classify("OOC_Reclaimer_Crash_Site_Daymar", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("crash_site"));
    }

    #[test]
    fn cave_synthetic_recognised() {
        let c = classify("LOC_Cave_Daymar_North", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("cave"));
    }

    #[test]
    fn bunker_synthetic_recognised() {
        let c = classify("Bunker_Hurston_Northwest", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("bunker"));
    }

    #[test]
    fn rest_stop_engine_prefix_classified_as_space_station_rest_stop() {
        // CIG ships `LOC_RR_S1_L3` for the R&R loadout at Stanton L3.
        let c = classify("LOC_RR_S1_L3", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert!(
            c.display_name.contains("Rest Stop"),
            "display: {}",
            c.display_name
        );
    }

    #[test]
    fn rest_stop_orbital_alias_resolves_port_tressler_from_catalog() {
        // Most recent LIVE case (2026-08-04): the inventory service emits
        // `RR_MIC_LEO` while the catalog row is named `port-tressler` and
        // carries no engine_tag. It must resolve to the named station, not
        // the generic "Rest Stop (Mic Leo)" fallback.
        let cat = catalog_with(vec![station_entry(
            "port-tressler",
            "Port Tressler",
            "microTech",
        )]);
        let c = classify("RR_MIC_LEO", &cat);
        assert_eq!(c.source, ClassificationSource::Catalog);
        assert_eq!(c.display_name, "Port Tressler");
        assert_eq!(c.slug.as_deref(), Some("port-tressler"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.parent_body.as_deref(), Some("microTech"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
    }

    #[test]
    fn rest_stop_external_alias_resolves_seraphim_station_from_catalog() {
        // Quantum targets use the `rs_ext_<body>-leo1` family for the
        // same orbital stations represented as `RR_<body>_LEO` by the
        // inventory service.
        let cat = catalog_with(vec![station_entry(
            "seraphim-station",
            "Seraphim Station",
            "Crusader",
        )]);
        let c = classify("rs_ext_cru-leo1", &cat);
        assert_eq!(c.source, ClassificationSource::Catalog);
        assert_eq!(c.display_name, "Seraphim Station");
        assert_eq!(c.slug.as_deref(), Some("seraphim-station"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
    }

    #[test]
    fn stanton_sector_s2_means_crusader_not_pyro() {
        // LIVE quantum target `LOC_RR_S2_L1`: S1-S4 are Stanton's body
        // sectors (HUR/CRU/ARC/MIC), not star-system numbers. The old
        // heuristic incorrectly treated S2 as Pyro.
        let cat = catalog_with(vec![station_entry(
            "cru-l1-ambitious-dream-station",
            "CRU-L1 Ambitious Dream Station",
            "Stanton",
        )]);
        let c = classify("LOC_RR_S2_L1", &cat);
        assert_eq!(c.source, ClassificationSource::Catalog);
        assert_eq!(c.slug.as_deref(), Some("cru-l1-ambitious-dream-station"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
    }

    #[test]
    fn stanton_station_alias_table_covers_every_catalogued_station() {
        let cases = [
            ("HUR", "LEO", "everus-harbor"),
            ("CRU", "LEO", "seraphim-station"),
            ("ARC", "LEO", "baijini-point"),
            ("MIC", "LEO", "port-tressler"),
            ("HUR", "L1", "hur-l1-green-glade-station"),
            ("HUR", "L2", "hur-l2-faithful-dream-station"),
            ("HUR", "L3", "hur-l3-thundering-express-station"),
            ("HUR", "L4", "hur-l4-melodic-fields-station"),
            ("HUR", "L5", "hur-l5-high-course-station"),
            ("CRU", "L1", "cru-l1-ambitious-dream-station"),
            ("CRU", "L4", "cru-l4-shallow-fields-station"),
            ("CRU", "L5", "cru-l5-beautiful-glen-station"),
            ("ARC", "L1", "arc-l1-wide-forest-station"),
            ("ARC", "L2", "arc-l2-lively-pathway-station"),
            ("ARC", "L3", "arc-l3-modern-express-station"),
            ("ARC", "L4", "arc-l4-faint-glen-station"),
            ("ARC", "L5", "arc-l5-yellow-core-station"),
            ("MIC", "L1", "mic-l1-shallow-frontier-station"),
            ("MIC", "L2", "mic-l2-long-forest-station"),
            ("MIC", "L3", "mic-l3-endless-odyssey-station"),
            ("MIC", "L4", "mic-l4-red-crossroads-station"),
            ("MIC", "L5", "mic-l5-modern-icarus-station"),
        ];

        for (sector, slot, expected) in cases {
            assert_eq!(
                stanton_station_slug(sector, slot),
                Some(expected),
                "sector={sector} slot={slot}"
            );
        }

        // The numeric sector family used by quantum targets maps to the
        // same body prefixes as the inventory aliases.
        assert_eq!(stanton_station_slug("S1", "L1"), cases[4].2.into());
        assert_eq!(stanton_station_slug("S2", "L1"), cases[9].2.into());
        assert_eq!(stanton_station_slug("S3", "L1"), cases[12].2.into());
        assert_eq!(stanton_station_slug("S4", "L1"), cases[17].2.into());
    }

    #[test]
    fn known_station_alias_without_catalog_row_keeps_generic_fallback() {
        // The alias table identifies which catalog row to use; it must not
        // fabricate a canonical name/link when an old or offline snapshot
        // does not contain that row.
        let c = classify("RR_MIC_LEO", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert!(c.slug.is_none());
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
        assert!(c.display_name.starts_with("Rest Stop"));
    }

    #[test]
    fn rest_stop_engine_prefix_handles_stanton_sector_short_form() {
        let c = classify("LOC_RR_S2_L1", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.parent_body.as_deref(), Some("Crusader"));
    }

    #[test]
    fn rest_stop_rs_ext_pyro_form_classifies_without_inventing_a_name() {
        // LIVE quantum target. Pyro station-number-to-name mappings are not
        // proven, but the engine shape and system are: retain a generic
        // station classification instead of falling all the way to Fallback.
        let c = classify("rs_ext_pyro3_l1", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
        assert_eq!(c.system.as_deref(), Some("Pyro"));
        assert_eq!(c.parent_body.as_deref(), Some("Bloom"));
        assert!(c.slug.is_none());
        assert!(c.display_name.starts_with("Rest Stop"));
    }

    #[test]
    fn pyro_orbital_aliases_resolve_catalog_stations() {
        let cat = catalog_with(vec![
            station_entry_in_system("orbituary", "Orbituary", "Pyro", "Bloom"),
            station_entry_in_system("ruin-station", "Ruin Station", "Pyro", "Terminus"),
        ]);

        let orbituary = classify("RR_P3_LEO", &cat);
        assert_eq!(orbituary.source, ClassificationSource::Catalog);
        assert_eq!(orbituary.display_name, "Orbituary");
        assert_eq!(orbituary.slug.as_deref(), Some("orbituary"));
        assert_eq!(orbituary.system.as_deref(), Some("Pyro"));
        assert_eq!(orbituary.parent_body.as_deref(), Some("Bloom"));
        assert_eq!(orbituary.tier, LocationTier::SpaceStation);
        assert_eq!(orbituary.subtype.as_deref(), Some("rest_stop"));

        let ruin = classify("RR_P6_LEO", &cat);
        assert_eq!(ruin.source, ClassificationSource::Catalog);
        assert_eq!(ruin.display_name, "Ruin Station");
        assert_eq!(ruin.slug.as_deref(), Some("ruin-station"));
        assert_eq!(ruin.system.as_deref(), Some("Pyro"));
        assert_eq!(ruin.parent_body.as_deref(), Some("Terminus"));
        assert_eq!(ruin.tier, LocationTier::SpaceStation);

        // Quantum/object-container variants split `Pyro3`/`Pyro6` into
        // separate tokens during normalization but identify the same orbitals.
        let orbituary_external = classify("rs_ext_pyro3_leo", &cat);
        assert_eq!(orbituary_external.slug.as_deref(), Some("orbituary"));
        assert_eq!(orbituary_external.display_name, "Orbituary");
        assert_eq!(orbituary_external.parent_body.as_deref(), Some("Bloom"));

        let ruin_external = classify("rs_ext_pyro6_leo", &cat);
        assert_eq!(ruin_external.slug.as_deref(), Some("ruin-station"));
        assert_eq!(ruin_external.display_name, "Ruin Station");
        assert_eq!(ruin_external.parent_body.as_deref(), Some("Terminus"));
    }

    #[test]
    fn pyro_body_codes_enrich_generic_rest_stops() {
        let cases = [
            ("RR_P1_L2", "Pyro I"),
            ("RR_P2_L4", "Monox"),
            ("RR_P3_L1", "Bloom"),
            ("RR_P4_L1", "Pyro IV"),
            ("RR_P5_L4", "Pyro V"),
            ("RR_P6_L5", "Terminus"),
        ];

        for (raw, expected_body) in cases {
            let c = classify(raw, &empty_catalog());
            assert_eq!(c.source, ClassificationSource::Synthetic, "raw={raw}");
            assert_eq!(c.system.as_deref(), Some("Pyro"), "raw={raw}");
            assert_eq!(c.parent_body.as_deref(), Some(expected_body), "raw={raw}");
            assert_eq!(c.tier, LocationTier::SpaceStation, "raw={raw}");
            assert_eq!(c.subtype.as_deref(), Some("rest_stop"), "raw={raw}");
        }
    }

    #[test]
    fn nyx_qv_extraction_station_family_gets_named_without_guessing_slug() {
        // Multiple LIVE keys (`004`, `033`, `035`, `048`) exist, while the
        // catalog has dozens of indistinguishable QV Breaker rows and no
        // engine tags. Name the family, but do not fabricate a specific link.
        let c = classify("Nyx_TSG_QVExtractionStation_035", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.display_name, "QV Breaker Station");
        assert!(c.slug.is_none());
        assert_eq!(c.system.as_deref(), Some("Nyx"));
        assert_eq!(c.parent_body.as_deref(), Some("Nyx"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert_eq!(c.subtype.as_deref(), Some("breaker_station"));
    }

    #[test]
    fn station_external_alias_without_catalog_stays_a_generic_station() {
        let c = classify("rs_ext_cru-leo1", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.parent_body.as_deref(), Some("Crusader"));
        assert!(c.slug.is_none());
    }

    #[test]
    fn rest_stop_engine_prefix_without_system_token_still_classifies() {
        // No system in the tail — we still tag as rest_stop with no
        // system populated rather than falling through.
        let c = classify("LOC_RR_GenericLoadout", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert!(c.system.is_none());
    }

    #[test]
    fn rest_stop_does_not_fire_on_substring_of_other_token() {
        // `BARRRACKS_Daymar` contains "rr" as a substring but no
        // standalone `RR` token. Must not trigger the matcher.
        let c = classify("BARRRACKS_Daymar", &empty_catalog());
        assert_ne!(c.subtype.as_deref(), Some("rest_stop"));
    }

    #[test]
    fn orbital_marker_with_body_and_index() {
        // CIG emits orbital markers around each body, e.g.
        // `OM_Hurston_3` for OM-3 around Hurston.
        let c = classify("OM_Hurston_3", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("orbital_marker"));
        assert_eq!(c.tier, LocationTier::AnonymousPoi);
        assert_eq!(c.parent_body.as_deref(), Some("Hurston"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert!(
            c.display_name.contains("OM-3"),
            "display: {}",
            c.display_name
        );
        assert_eq!(
            c.placement,
            Some(Placement::OrbitsBody {
                body: "Hurston".to_string()
            })
        );
    }

    #[test]
    fn orbital_marker_combined_om_index_form() {
        // `OM-1_Daymar` and `OM1_Daymar` (no separator) both parse.
        let c1 = classify("OM-1_Daymar", &empty_catalog());
        assert_eq!(c1.subtype.as_deref(), Some("orbital_marker"));
        assert_eq!(c1.parent_body.as_deref(), Some("Daymar"));

        let c2 = classify("OM1_Daymar", &empty_catalog());
        assert_eq!(c2.subtype.as_deref(), Some("orbital_marker"));
        assert_eq!(c2.parent_body.as_deref(), Some("Daymar"));
    }

    #[test]
    fn orbital_marker_without_body_still_classifies() {
        let c = classify("LOC_OM_4", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("orbital_marker"));
        assert!(c.parent_body.is_none());
        // Display still surfaces the index even without a body.
        assert!(
            c.display_name.contains("OM-4"),
            "display: {}",
            c.display_name
        );
    }

    #[test]
    fn orbital_marker_does_not_fire_on_words_containing_om() {
        // `Lorville` contains "om" as a substring but no standalone
        // OM token. Must not trigger the matcher.
        let c = classify("OOC_Stanton_3_Lorville", &empty_catalog());
        assert_ne!(c.subtype.as_deref(), Some("orbital_marker"));
    }

    // ---- gaps surfaced by a real LIVE log audit (2026-05-26) -------
    //
    // Every test below was added after running the classifier
    // against an actual gameplay log; each name preserves the
    // engine identifier as it appeared in the log so future
    // regressions land in the same file.

    #[test]
    fn ariel_legacy_engine_spelling_maps_to_canonical_arial() {
        // Engine retains the pre-3.3.0 `Ariel` spelling; the wiki
        // and our taxonomy use `Arial`. Both must resolve.
        let c = classify("OOC_Stanton_1a_Ariel", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Heuristic);
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.display_name, "Arial");
    }

    #[test]
    fn rest_stop_engine_prefix_infers_system_from_body_short_code() {
        // Real engine identifier `RR_MIC_LEO` — no system token, but
        // `MIC` short-codes microTech which lives in Stanton. The
        // matcher must propagate that system + parent_body.
        let c = classify("RR_MIC_LEO", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.parent_body.as_deref(), Some("microTech"));
    }

    #[test]
    fn jump_point_bare_system_form() {
        // `magnus_jp1` / `stan_jp1` / `pyro_jp1` — engine emits
        // these for the four-pillar jump network without any `rs_`
        // prefix at all. Must still classify as jump_point.
        for raw in &["magnus_jp1", "stan_jp1", "pyro_jp1", "terra_jp1"] {
            let c = classify(raw, &empty_catalog());
            assert_eq!(
                c.subtype.as_deref(),
                Some("jump_point"),
                "raw={raw} display={}",
                c.display_name
            );
            assert_eq!(c.tier, LocationTier::AnonymousPoi);
        }
    }

    #[test]
    fn jump_point_rs_entry_and_rs_comm_verbs() {
        // CIG's engine ships at least `rs_entry`, `rs_comm`, and
        // `rs_ext` as the verb after `rs`. The matcher used to gate
        // strictly on `rs_ext`, which dropped both `entry` and
        // `comm` — both observed in real LIVE logs.
        let c = classify("rs_entry_nyx_pyro_jp1", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("jump_point"));
        assert!(
            c.display_name.to_lowercase().contains("nyx")
                || c.display_name.to_lowercase().contains("pyro"),
            "expected endpoint in display, got {}",
            c.display_name
        );

        let c2 = classify("rs_comm_nyx_castra_jp1", &empty_catalog());
        assert_eq!(c2.subtype.as_deref(), Some("jump_point"));
    }

    #[test]
    fn jump_point_hyphenated_endpoints_still_work() {
        // Existing shape — must not regress.
        let c = classify("LOC_rs_ext_stan-pyro_jp1", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("jump_point"));
        assert!(c.display_name.contains("Stan"));
        assert!(c.display_name.contains("Pyro"));
    }

    #[test]
    fn rest_stop_generic_token() {
        // `ObjectContainer_RestStop` — emitted when the player
        // quantum-targets a generic "nearest rest stop" rather than
        // a specific catalogued station. Should tag as rest_stop
        // tier=SpaceStation, no system/body context expected.
        let c = classify("ObjectContainer_RestStop", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("rest_stop"));
        assert_eq!(c.tier, LocationTier::SpaceStation);
    }

    #[test]
    fn area18_city_object_container_resolves() {
        // `Area18_City_objectContainer` — the engine identifier for
        // ArcCorp's Area18 landing zone. Without a catalog hit, the
        // matcher should at least surface Stanton + Area18 via the
        // body-short-code dictionary entry.
        let c = classify("Area18_City_objectContainer", &empty_catalog());
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.display_name, "Area18");
    }

    // ---- heuristic fallbacks ---------------------------------------

    #[test]
    fn body_short_code_resolves_system_and_body() {
        // HUR_L1 with no catalogue match — fall back to heuristic.
        let c = classify("HUR_L1_Faithful_Dream", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Heuristic);
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.display_name, "Hurston");
    }

    #[test]
    fn bare_system_fallback_when_no_body() {
        let c = classify("Stanton_NewUnknownPlace", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Heuristic);
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.tier, LocationTier::AnonymousPoi);
    }

    #[test]
    fn unknown_string_falls_back_to_title_case() {
        let c = classify("WhatIsThisEvenSupposedToBe", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Fallback);
        assert_eq!(c.tier, LocationTier::AnonymousPoi);
        assert!(!c.display_name.is_empty());
    }

    #[test]
    fn empty_input_falls_back_cleanly() {
        let c = classify("", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Fallback);
        assert!(c.display_name.is_empty());
    }

    // ---- strip_and_split -------------------------------------------

    #[test]
    fn strip_and_split_drops_runtime_prefix_and_skip_segments() {
        let parts = strip_and_split("[PROC]OOC_Stanton_3_Lorville");
        assert_eq!(parts, vec!["Stanton", "3", "Lorville"]);
    }

    #[test]
    fn strip_and_split_unjoins_system_index() {
        let parts = strip_and_split("Stanton2_Orison_LOC");
        assert_eq!(parts, vec!["Stanton", "2", "Orison"]);

        let parts = strip_and_split("Pyro4a_Vatra");
        assert_eq!(parts, vec!["Pyro", "4a", "Vatra"]);
    }

    #[test]
    fn strip_and_split_handles_only_skip_segments() {
        let parts = strip_and_split("OOC_LOC_PROC");
        assert!(parts.is_empty());
    }

    // ---- classification_to_tier ------------------------------------

    #[test]
    fn classification_to_tier_handles_known_strings() {
        assert_eq!(
            classification_to_tier(Some("Planet")),
            Some(LocationTier::AstronomicalObject)
        );
        assert_eq!(
            classification_to_tier(Some("space station")),
            Some(LocationTier::SpaceStation)
        );
        assert_eq!(
            classification_to_tier(Some("Settlement")),
            Some(LocationTier::Landmark)
        );
        assert_eq!(classification_to_tier(None), None);
        assert_eq!(classification_to_tier(Some("brand new wiki bucket")), None);
    }

    // ---- catalog beats heuristic -----------------------------------

    #[test]
    fn catalog_beats_heuristic_when_both_match() {
        // `Stanton` would otherwise heuristic-route via KNOWN_SYSTEMS;
        // an Aberdeen catalog hit should win because it comes first
        // in the resolution order.
        let cat = catalog_with(vec![aberdeen_entry()]);
        let c = classify("OOC_Stanton_1b_Aberdeen", &cat);
        assert_eq!(c.source, ClassificationSource::Catalog);
        assert_eq!(c.display_name, "Aberdeen");
    }

    #[test]
    fn specific_body_wins_over_bare_system_token() {
        // Regression guard for the shadowing fix: with BOTH the Stanton
        // system row and the Daymar moon in the catalogue, the engine
        // string `OOC_Stanton_2b_Daymar` must resolve to "Daymar", not
        // be shadowed by the leading `Stanton` token. Pre-fix this
        // collapsed 91% of real location events to their system name.
        let daymar = LocationCatalogEntry {
            slug: "daymar".into(),
            display_name: "Daymar".into(),
            class_name: "daymar".into(),
            engine_tag: None,
            system: Some("Stanton".into()),
            parent_body: Some("Crusader".into()),
            classification: Some("Moon".into()),
            taxonomy: LocationTaxonomy {
                tier: Some(LocationTier::AstronomicalObject),
                subtype: Some("moon".into()),
                ..LocationTaxonomy::default()
            },
        };
        let stanton_system = LocationCatalogEntry {
            slug: "stanton".into(),
            display_name: "Stanton".into(),
            class_name: "stanton".into(),
            engine_tag: None,
            system: Some("Stanton".into()),
            parent_body: None,
            classification: Some("System".into()),
            taxonomy: LocationTaxonomy::default(),
        };
        let cat = catalog_with(vec![stanton_system, daymar]);
        let c = classify("OOC_Stanton_2b_Daymar", &cat);
        assert_eq!(c.display_name, "Daymar");
        assert_eq!(c.source, ClassificationSource::Catalog);
        assert_eq!(c.subtype.as_deref(), Some("moon"));
    }

    #[test]
    fn bare_system_identifier_still_resolves_to_system_row() {
        // The flip side of the shadowing fix: a *bare* system string
        // (no specific body token) must still hit the system row via
        // the deferred second pass — keeping its slug + System tier.
        let stanton_system = LocationCatalogEntry {
            slug: "stanton".into(),
            display_name: "Stanton".into(),
            class_name: "stanton".into(),
            engine_tag: None,
            system: Some("Stanton".into()),
            parent_body: None,
            classification: Some("System".into()),
            taxonomy: LocationTaxonomy {
                tier: Some(LocationTier::System),
                ..LocationTaxonomy::default()
            },
        };
        let cat = catalog_with(vec![stanton_system]);
        let c = classify("OOC_Stanton", &cat);
        assert_eq!(c.display_name, "Stanton");
        assert_eq!(c.source, ClassificationSource::Catalog);
        assert_eq!(c.tier, LocationTier::System);
    }

    // ---- distinctive-token fuzzy matcher ---------------------------
    //
    // Every engine identifier below is verbatim from a real LIVE tray
    // DB (2026-05-31); the wiki names are the real
    // api.star-citizen.wiki display names they should resolve to.
    // These are the "real but unmatched by exact keys" locations that
    // motivated the fuzzy tier.

    fn outpost(slug: &str, name: &str, system: &str) -> LocationCatalogEntry {
        LocationCatalogEntry {
            slug: slug.into(),
            display_name: name.into(),
            class_name: name.replace(' ', ""),
            engine_tag: None,
            system: Some(system.into()),
            parent_body: None,
            classification: Some("Outpost".into()),
            taxonomy: LocationTaxonomy {
                tier: Some(LocationTier::Landmark),
                subtype: Some("outpost".into()),
                ..LocationTaxonomy::default()
            },
        }
    }

    /// A realistic-ish catalog: the real recoverable rows plus enough
    /// filler "* Research Outpost" rows that `outpost` / `research` /
    /// `mining` climb above `FUZZY_ANCHOR_DF`, so a filler-only overlap is
    /// correctly rejected (mirrors the real ~1955-row catalogue).
    fn fuzzy_catalog() -> LocationCatalog {
        let mut entries = vec![
            outpost(
                "rayari-kaltag-research-outpost",
                "Rayari Kaltag Research Outpost",
                "Stanton",
            ),
            outpost(
                "rayari-deltana-research-outpost",
                "Rayari Deltana Research Outpost",
                "Stanton",
            ),
            outpost(
                "rayari-cantwell-research-outpost",
                "Rayari Cantwell Research Outpost",
                "Stanton",
            ),
            outpost(
                "rayari-anvik-research-outpost",
                "Rayari Anvik Research Outpost",
                "Stanton",
            ),
            // Fifth Rayari outpost → `rayari` df 5 (> FUZZY_ANCHOR_DF),
            // so the operator name alone can never anchor a match.
            outpost(
                "rayari-hickes-research-outpost",
                "Rayari Hickes Research Outpost",
                "Stanton",
            ),
            // Coincidental rare `hydro` token — the trap that must NOT
            // catch `RayariHydro_*` engine strings.
            outpost("terra-mills-hydrofarm", "Terra Mills HydroFarm", "Stanton"),
            outpost(
                "shubin-mining-facility-sal-2",
                "Shubin Mining Facility SAL-2",
                "Stanton",
            ),
            outpost(
                "shubin-mining-facility-sal-5",
                "Shubin Mining Facility SAL-5",
                "Stanton",
            ),
            outpost(
                "sakura-sun-goldenrod-workcenter",
                "Sakura Sun Goldenrod Workcenter",
                "Stanton",
            ),
            outpost("benson-mining-outpost", "Benson Mining Outpost", "Stanton"),
            outpost(
                "deakins-research-outpost",
                "Deakins Research Outpost",
                "Stanton",
            ),
        ];
        // Filler padding — inflate df of generic words (`research`,
        // `outpost`) so a filler-only overlap can't clear the anchor.
        for i in 0..12 {
            entries.push(outpost(
                &format!("filler-{i}-research-outpost"),
                &format!("Filler{i} Research Outpost"),
                "Stanton",
            ));
        }
        catalog_with(entries)
    }

    #[test]
    fn fuzzy_recovers_rayari_kaltag() {
        let cat = fuzzy_catalog();
        let c = classify("Stanton4a_RayariHydro_Kaltag", &cat);
        assert_eq!(c.source, ClassificationSource::Fuzzy);
        assert_eq!(c.slug.as_deref(), Some("rayari-kaltag-research-outpost"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
    }

    #[test]
    fn fuzzy_recovers_sakura_sun_goldenrod() {
        let cat = fuzzy_catalog();
        let c = classify("Stanton4_DistributionCentre_SakuraSun_Goldenrod", &cat);
        assert_eq!(c.source, ClassificationSource::Fuzzy);
        assert_eq!(c.slug.as_deref(), Some("sakura-sun-goldenrod-workcenter"));
    }

    #[test]
    fn fuzzy_disambiguates_shubin_sal2_from_sal5() {
        // Both rows share `shubin`+`mining`+`facility`+`sal`; only the
        // trailing digit separates them. The idf score must tip toward
        // the row that also shares the `2`. (Here `shubin` df is 2,
        // within FUZZY_ANCHOR_DF, so it anchors; in the full
        // production catalogue `shubin` is more common and this family
        // falls back to the system heuristic — a deliberate
        // precision-over-recall trade for digit-only discriminators.)
        let cat = fuzzy_catalog();
        let c2 = classify("Stanton3a_Shubin_SAL2", &cat);
        assert_eq!(c2.slug.as_deref(), Some("shubin-mining-facility-sal-2"));
        let c5 = classify("Stanton3a_Shubin_SAL5", &cat);
        assert_eq!(c5.slug.as_deref(), Some("shubin-mining-facility-sal-5"));
    }

    #[test]
    fn fuzzy_rejects_filler_only_overlap() {
        // Engine string shares only `research`/`outpost` (both far
        // above FUZZY_ANCHOR_DF). No distinctive anchor → no fuzzy hit;
        // falls through to the system heuristic instead of fabricating
        // a wrong wiki link.
        let cat = fuzzy_catalog();
        let c = classify("Stanton2a_Unmapped_Research_Outpost", &cat);
        assert_ne!(c.source, ClassificationSource::Fuzzy);
        assert_eq!(c.system.as_deref(), Some("Stanton"));
    }

    #[test]
    fn fuzzy_rejects_uncatalogued_place_with_only_operator_overlap() {
        // `RayariHydro_McGarth`: there is no `McGarth` row. The engine
        // string overlaps the catalogue only on the operator `rayari`
        // (df 5, above the anchor bar) and the affiliation word `hydro`
        // (denylisted). Neither may anchor → no match, so it must NOT
        // bind to a random Rayari sibling or to Terra Mills HydroFarm.
        let cat = fuzzy_catalog();
        let c = classify("Stanton4b_RayariHydro_McGarth", &cat);
        assert_ne!(
            c.source,
            ClassificationSource::Fuzzy,
            "unexpected fuzzy bind to {}",
            c.display_name
        );
    }

    #[test]
    fn fuzzy_respects_system_consistency_guard() {
        // The only `Kaltag` row is in Pyro; a Stanton engine string
        // must NOT cross-match it.
        let cat = catalog_with(vec![outpost(
            "rayari-kaltag-research-outpost",
            "Rayari Kaltag Research Outpost",
            "Pyro",
        )]);
        let c = classify("Stanton4a_RayariHydro_Kaltag", &cat);
        assert_ne!(c.source, ClassificationSource::Fuzzy);
    }

    #[test]
    fn fuzzy_does_not_fire_when_exact_key_matches() {
        // Exact engine-tag/slug must always win over fuzzy.
        let cat = catalog_with(vec![aberdeen_entry()]);
        let c = classify("OOC_Stanton_1b_Aberdeen", &cat);
        assert_eq!(c.source, ClassificationSource::Catalog);
    }

    // ---- noise classification --------------------------------------

    #[test]
    fn noise_asteroid_mining_node() {
        let c = classify("ab_mine_stanton2_med_010", &empty_catalog());
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("mining_node"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
    }

    #[test]
    fn noise_gas_collection_node() {
        let c = classify("ab_collector_gas_Stanton1", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("gas_node"));
    }

    #[test]
    fn noise_asteroid_cluster_socpak_beats_fuzzy() {
        // `shubin_cluster_..._.socpak` is a procedural asteroid field,
        // NOT the Shubin facility. Noise classification (a synthetic
        // matcher) runs before fuzzy, so even with the facility in the
        // catalog it must classify as a cluster.
        let cat = fuzzy_catalog();
        let c = classify(
            "shubin_cluster_001_frost_{13DA184B-8620-4DAE-9450-5CE6F2ADA1A5}.socpak",
            &cat,
        );
        assert_eq!(c.source, ClassificationSource::Synthetic);
        assert_eq!(c.subtype.as_deref(), Some("asteroid_cluster"));
    }

    #[test]
    fn noise_mission_marker() {
        let c = classify("MISSION_QT_Quantum_Beacon_286174403838", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("mission_marker"));
    }

    #[test]
    fn noise_dynamic_nav_point() {
        let c = classify("NavPoint_Dynamic_285165357631", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("nav_marker"));
    }

    #[test]
    fn noise_race_track() {
        let c = classify("racing_static_st2c_ghexasteroid", &empty_catalog());
        assert_eq!(c.subtype.as_deref(), Some("race_track"));
    }
}
