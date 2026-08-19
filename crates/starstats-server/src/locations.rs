//! Location resolution from raw event payloads.
//!
//! Star Citizen sprinkles location signals across several different
//! events; this module collapses them into a single
//! `{ planet, city, system, shard, source_event_type }` shape so the
//! frontend can render a "you are here" pill without knowing which
//! variant of which event happened to fire most recently.
//!
//! ## Resolution priority
//!
//! When multiple location-bearing events have fired in a session, the
//! most precise wins:
//!
//! 1. `LocationInventoryRequested.location` — names a specific city
//!    (e.g. `Stanton2_Orison` → "Orison, Crusader"). Most precise.
//! 2. `PlanetTerrainLoad.planet` — names a celestial body
//!    (e.g. `OOC_Stanton_2b_Daymar` → "Daymar"). Less precise but
//!    fires more reliably.
//! 3. `JoinPu.shard` — only confirms "they're online in shard X" with
//!    no spatial info. Used as a last resort so an idle player still
//!    surfaces *something* rather than nothing.
//!
//! The handler queries the events table for the most recent event of
//! any of these three types per user, then funnels the row through
//! [`resolve`] to produce the wire DTO.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// One resolved location reading. `planet` and `city` are
/// human-readable labels; the raw engine identifiers (e.g.
/// `OOC_Stanton_2b_Daymar`) live alongside in `raw_*` so callers
/// can drill into the original event if they want.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ResolvedLocation {
    /// Human-readable celestial body, e.g. "Daymar". `None` when the
    /// most-recent event was a `JoinPu` (shard-only signal).
    pub planet: Option<String>,
    /// Specific landing zone / city when known, e.g. "Orison" or
    /// "Lorville". `None` for events that only carry planet-level
    /// info, or when the location doesn't correspond to a city we
    /// recognise.
    pub city: Option<String>,
    /// Star system, e.g. "Stanton" or "Pyro". Derived from the same
    /// raw key as the planet / city fields when possible.
    pub system: Option<String>,
    /// Shard id from the most-recent `JoinPu`, when one is present
    /// in the surrounding session. Carried independently of the
    /// planet/city fields so the UI can show "Daymar · shard
    /// pub_euw1b" together when both are available.
    pub shard: Option<String>,
    /// When the source event fired. Drives the staleness check on
    /// the read endpoint and the "last seen X minutes ago" UI label.
    pub last_seen_at: DateTime<Utc>,
    /// Which event variant supplied the location. One of:
    /// `location_inventory_requested`, `planet_terrain_load`,
    /// `join_pu`.
    pub source_event_type: String,
    /// Raw engine key (the unprocessed value from the source event)
    /// — kept around so a future "open this in starmap.tools" link
    /// has the original identifier to work with.
    pub raw_planet_key: Option<String>,
    pub raw_city_key: Option<String>,
    /// When the user first entered the current location (the oldest
    /// timestamp in the contiguous run of same-`location_key` events
    /// ending at `last_seen_at`). `None` when only a single
    /// location-bearing event is on record. Computed by the handler,
    /// not the resolver — see `query::location_current`.
    #[serde(default)]
    pub entered_at: Option<DateTime<Utc>>,
    /// `true` when the walk-back exhausted its batch without finding
    /// a key change — `entered_at` then represents the oldest event
    /// we *saw*, not necessarily the true entry. The UI surfaces
    /// this as a trailing `+` on the dwell display ("here 23h 57m+")
    /// so the user knows the real dwell is at least that long.
    #[serde(default)]
    pub entered_at_is_lower_bound: bool,
    /// Classifier output for the most-specific raw key, derived at query
    /// time by `query::location_current` from the `LocationCatalogCache`
    /// (`resolve` itself is catalog-free, so it leaves this `None`).
    /// Drives the friendly display name + `/kb/location/{slug}` link on
    /// the web "you are here" surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<crate::ingest::ResolvedLocationSchema>)]
    pub resolved_location: Option<starstats_core::location_classifier::ResolvedLocation>,
}

/// Maximum number of recent location-bearing events the handler
/// walks back through when computing `entered_at`. 500 covers
/// realistic single-location stays comfortably; past that we mark
/// the result as a lower bound rather than silently truncating.
pub const ENTERED_AT_RUN_LIMIT: i64 = 500;

/// Stable identity key for a resolved location — `system|planet|city`.
/// Two `ResolvedLocation`s with the same key represent the same place
/// for dwell-run purposes. Mirrors the web-side `locationKey` in
/// `apps/web/src/components/journey/trail-utils.ts` so the same notion
/// of "still here" applies on both sides.
pub fn location_key(loc: &ResolvedLocation) -> String {
    format!(
        "{}|{}|{}",
        loc.system.as_deref().unwrap_or(""),
        loc.planet.as_deref().unwrap_or(""),
        loc.city.as_deref().unwrap_or(""),
    )
}

/// Order matters: most precise first. Surfaced as a public const so
/// the repo's `latest_location` query can use the same canonical list
/// instead of hand-rolling its `IN (...)` clause.
pub const LOCATION_EVENT_TYPES: &[&str] = &[
    "location_inventory_requested",
    "planet_terrain_load",
    "join_pu",
];

/// Resolve a single event row into a [`ResolvedLocation`]. Returns
/// `None` when the payload doesn't carry the expected fields — that
/// shouldn't happen for events the parser produced, but ingest from
/// older client versions might have a different shape.
pub fn resolve(
    event_type: &str,
    payload: &Value,
    last_seen_at: DateTime<Utc>,
    shard_hint: Option<String>,
) -> Option<ResolvedLocation> {
    match event_type {
        "location_inventory_requested" => {
            resolve_inventory_request(payload, last_seen_at, shard_hint)
        }
        "planet_terrain_load" => resolve_planet_terrain(payload, last_seen_at, shard_hint),
        "join_pu" => resolve_join_pu(payload, last_seen_at),
        _ => None,
    }
}

fn resolve_inventory_request(
    payload: &Value,
    last_seen_at: DateTime<Utc>,
    shard_hint: Option<String>,
) -> Option<ResolvedLocation> {
    let raw_city = payload.get("location")?.as_str()?;
    // The engine emits `INVALID_LOCATION_ID` when the player isn't
    // bound to a known location yet (loading, deep space). We still
    // return a row so the staleness check sees a recent event, but
    // we drop the city/planet fields — the UI shows "in transit".
    if raw_city == "INVALID_LOCATION_ID" {
        return Some(ResolvedLocation {
            planet: None,
            city: None,
            system: None,
            shard: shard_hint,
            last_seen_at,
            source_event_type: "location_inventory_requested".to_string(),
            raw_planet_key: None,
            raw_city_key: Some(raw_city.to_string()),
            entered_at: None,
            entered_at_is_lower_bound: false,
            resolved_location: None,
        });
    }
    let parsed = parse_city_key(raw_city);
    Some(ResolvedLocation {
        planet: parsed.planet_label,
        city: parsed.city_label,
        system: parsed.system,
        shard: shard_hint,
        last_seen_at,
        source_event_type: "location_inventory_requested".to_string(),
        raw_planet_key: parsed.raw_planet_key,
        raw_city_key: Some(raw_city.to_string()),
        entered_at: None,
        entered_at_is_lower_bound: false,
        resolved_location: None,
    })
}

fn resolve_planet_terrain(
    payload: &Value,
    last_seen_at: DateTime<Utc>,
    shard_hint: Option<String>,
) -> Option<ResolvedLocation> {
    let raw_planet = payload.get("planet")?.as_str()?;
    let parsed = parse_planet_key(raw_planet);
    Some(ResolvedLocation {
        planet: parsed.planet_label,
        city: None,
        system: parsed.system,
        shard: shard_hint,
        last_seen_at,
        source_event_type: "planet_terrain_load".to_string(),
        raw_planet_key: Some(raw_planet.to_string()),
        raw_city_key: None,
        entered_at: None,
        entered_at_is_lower_bound: false,
        resolved_location: None,
    })
}

fn resolve_join_pu(payload: &Value, last_seen_at: DateTime<Utc>) -> Option<ResolvedLocation> {
    let shard = payload.get("shard")?.as_str()?.to_string();
    Some(ResolvedLocation {
        planet: None,
        city: None,
        system: system_from_shard(&shard),
        shard: Some(shard),
        last_seen_at,
        source_event_type: "join_pu".to_string(),
        raw_planet_key: None,
        raw_city_key: None,
        entered_at: None,
        entered_at_is_lower_bound: false,
        resolved_location: None,
    })
}

/// Best-effort mapping from a shard identifier to a star system. The
/// shard naming today doesn't actually carry system info reliably
/// (`pub_euw1b_<id>_<server>`), so this returns `None` until CIG
/// changes the format. Kept as a hook so a future "Pyro shards have
/// `pyro` in the name" change doesn't need a code-shape change.
fn system_from_shard(_shard: &str) -> Option<String> {
    None
}

/// Decomposed city-key parse result. Internal — callers go through
/// [`resolve_inventory_request`].
struct CityParse {
    planet_label: Option<String>,
    city_label: Option<String>,
    system: Option<String>,
    raw_planet_key: Option<String>,
}

/// City keys observed in the wild look like `Stanton2_Orison`
/// (i.e. `<System><PlanetIndex>_<CityName>`). We pattern-match
/// against a small dictionary of known cities, then fall back to
/// stripping the prefix to recover an at-least-readable name. Keep
/// the dictionary tight — only the cities we've actually seen in
/// captures should produce a confident planet attribution.
fn parse_city_key(raw: &str) -> CityParse {
    // Hand-rolled table because (a) it's tiny and (b) CIG occasionally
    // adds a city per patch — adding a row here is the canonical place
    // to teach the parser about it.
    const KNOWN_CITIES: &[(&str, &str, &str, &str, &str)] = &[
        // (raw_key, system, planet_label, city_label, raw_planet_key)
        (
            "Stanton1_Lorville",
            "Stanton",
            "Hurston",
            "Lorville",
            "OOC_Stanton_1_Hurston",
        ),
        (
            "Stanton2_Orison",
            "Stanton",
            "Crusader",
            "Orison",
            "OOC_Stanton_2_Crusader",
        ),
        (
            "Stanton3_Area18",
            "Stanton",
            "ArcCorp",
            "Area18",
            "OOC_Stanton_3_ArcCorp",
        ),
        (
            "Stanton4_NewBabbage",
            "Stanton",
            "microTech",
            "New Babbage",
            "OOC_Stanton_4_microTech",
        ),
        // Pyro cities — names taken from the live game; if CIG renames
        // we update here.
        ("Pyro1_Ruin", "Pyro", "Pyro I", "Ruin Station", "OOC_Pyro_1"),
        (
            "Pyro5_Checkmate",
            "Pyro",
            "Pyro V",
            "Checkmate",
            "OOC_Pyro_5",
        ),
    ];

    if let Some(&(_, system, planet, city, planet_key)) =
        KNOWN_CITIES.iter().find(|(k, _, _, _, _)| *k == raw)
    {
        return CityParse {
            planet_label: Some(planet.to_string()),
            city_label: Some(city.to_string()),
            system: Some(system.to_string()),
            raw_planet_key: Some(planet_key.to_string()),
        };
    }

    // Fallback: split on the first underscore. Surface whatever's
    // after it as a best-effort city label so the user sees something
    // useful even for cities we haven't catalogued.
    if let Some((system_part, city_part)) = raw.split_once('_') {
        return CityParse {
            planet_label: None,
            city_label: Some(humanise(city_part)),
            system: extract_system_prefix(system_part),
            raw_planet_key: None,
        };
    }

    CityParse {
        planet_label: None,
        city_label: Some(humanise(raw)),
        system: None,
        raw_planet_key: None,
    }
}

#[derive(Debug)]
struct PlanetParse {
    planet_label: Option<String>,
    system: Option<String>,
}

/// Planet keys look like `OOC_Stanton_2b_Daymar` or
/// `OOC_Pyro_1_<unnamed>`. We split on `_` and pick the meaningful
/// segments. The trailing label is usually the celestial name; the
/// second segment is the system.
fn parse_planet_key(raw: &str) -> PlanetParse {
    let cleaned = raw.trim_start_matches("OOC_");
    let parts: Vec<&str> = cleaned.split('_').collect();
    let system = parts.first().map(|s| s.to_string());
    // Most planet keys end with the human name. Fall back to the full
    // cleaned string if there's only one segment.
    let planet_label = parts.last().map(|s| humanise(s));
    PlanetParse {
        planet_label,
        system,
    }
}

/// Convert an engine token (typically PascalCase or snake_case with
/// no spaces) to a display string. Doesn't try to be clever — we
/// just inject spaces before internal capitals and capitalise the
/// first letter. CIG names are already mostly idiomatic so this is
/// usually a no-op.
fn humanise(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for (i, ch) in raw.chars().enumerate() {
        if i > 0 && ch.is_ascii_uppercase() {
            // Don't insert a space if the previous char was already
            // uppercase (avoids breaking "ArcCorp" into "Arc Corp",
            // which while arguably more readable is not what CIG
            // names it). The dictionary above wins for known cases;
            // this is the unknown-fallback path.
            if let Some(prev) = out.chars().last() {
                if !prev.is_ascii_uppercase() {
                    out.push(' ');
                }
            }
        }
        out.push(ch);
    }
    if let Some(c) = out.chars().next() {
        if c.is_ascii_lowercase() {
            // Capitalise first char only when it's currently lower —
            // preserves names like "microTech" that intentionally
            // start lowercase.
            // Actually the original may legitimately be lowercase, so
            // skip the auto-capitalise. The dictionary handles known
            // cases; bare keys are at-most-readable as-is.
        }
    }
    out
}

/// Extract a system label from a city-key prefix like `Stanton2`.
/// We trim trailing digits to recover `Stanton`. Returns `None` if
/// the prefix is empty after trimming.
fn extract_system_prefix(prefix: &str) -> Option<String> {
    let trimmed = prefix.trim_end_matches(char::is_numeric);
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ts() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-06T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn resolves_known_city_with_planet_attribution() {
        let payload = json!({"location": "Stanton2_Orison", "player": "alice"});
        let r = resolve("location_inventory_requested", &payload, ts(), None).unwrap();
        assert_eq!(r.city.as_deref(), Some("Orison"));
        assert_eq!(r.planet.as_deref(), Some("Crusader"));
        assert_eq!(r.system.as_deref(), Some("Stanton"));
        assert_eq!(r.source_event_type, "location_inventory_requested");
    }

    #[test]
    fn resolves_unknown_city_with_humanised_fallback() {
        let payload = json!({"location": "Stanton9_NewMystery"});
        let r = resolve("location_inventory_requested", &payload, ts(), None).unwrap();
        assert!(r.city.is_some(), "expected fallback city label");
        // Planet attribution is None for unknown cities — we only
        // claim a planet when the dictionary maps it explicitly.
        assert!(r.planet.is_none());
        assert_eq!(r.system.as_deref(), Some("Stanton"));
    }

    #[test]
    fn resolves_invalid_location_id_to_in_transit_shape() {
        let payload = json!({"location": "INVALID_LOCATION_ID"});
        let r = resolve("location_inventory_requested", &payload, ts(), None).unwrap();
        assert!(r.planet.is_none());
        assert!(r.city.is_none());
        assert!(r.system.is_none());
        // last_seen_at still flows so the freshness check on the
        // endpoint sees activity.
        assert_eq!(r.last_seen_at, ts());
    }

    #[test]
    fn resolves_planet_terrain_strips_ooc_prefix() {
        let payload = json!({"planet": "OOC_Stanton_2b_Daymar"});
        let r = resolve("planet_terrain_load", &payload, ts(), None).unwrap();
        assert_eq!(r.planet.as_deref(), Some("Daymar"));
        assert_eq!(r.system.as_deref(), Some("Stanton"));
        assert!(r.city.is_none());
        assert_eq!(
            r.raw_planet_key.as_deref(),
            Some("OOC_Stanton_2b_Daymar"),
            "raw key preserved for caller"
        );
    }

    #[test]
    fn resolves_planet_terrain_with_pyro_system() {
        let payload = json!({"planet": "OOC_Pyro_5_Monox"});
        let r = resolve("planet_terrain_load", &payload, ts(), None).unwrap();
        assert_eq!(r.system.as_deref(), Some("Pyro"));
        assert_eq!(r.planet.as_deref(), Some("Monox"));
    }

    #[test]
    fn resolves_join_pu_to_shard_only_shape() {
        let payload = json!({
            "shard": "pub_euw1b_11704877_090",
            "address": "1.2.3.4",
            "port": 64300,
            "location_id": "1"
        });
        let r = resolve("join_pu", &payload, ts(), None).unwrap();
        assert!(r.planet.is_none());
        assert!(r.city.is_none());
        assert_eq!(r.shard.as_deref(), Some("pub_euw1b_11704877_090"));
        assert_eq!(r.source_event_type, "join_pu");
    }

    #[test]
    fn resolve_returns_none_for_unrelated_event_types() {
        let payload = json!({"timestamp": "x"});
        assert!(resolve("actor_death", &payload, ts(), None).is_none());
        assert!(resolve("hud_notification", &payload, ts(), None).is_none());
    }

    #[test]
    fn resolve_returns_none_when_payload_missing_field() {
        let payload = json!({"unrelated": "field"});
        assert!(resolve("planet_terrain_load", &payload, ts(), None).is_none());
        assert!(resolve("location_inventory_requested", &payload, ts(), None).is_none());
        assert!(resolve("join_pu", &payload, ts(), None).is_none());
    }

    #[test]
    fn location_key_matches_for_same_system_planet_city() {
        let payload = json!({"location": "Stanton4_NewBabbage"});
        let a = resolve("location_inventory_requested", &payload, ts(), None).unwrap();
        let b = resolve(
            "location_inventory_requested",
            &payload,
            ts() + chrono::Duration::minutes(30),
            None,
        )
        .unwrap();
        // Same place at different timestamps → same key. This is what
        // the run-walk in `location_current` compares to decide whether
        // the user is still in the same location.
        assert_eq!(location_key(&a), location_key(&b));
    }

    #[test]
    fn location_key_differs_when_planet_changes() {
        let new_babbage = resolve(
            "location_inventory_requested",
            &json!({"location": "Stanton4_NewBabbage"}),
            ts(),
            None,
        )
        .unwrap();
        let lorville = resolve(
            "location_inventory_requested",
            &json!({"location": "Stanton1_Lorville"}),
            ts(),
            None,
        )
        .unwrap();
        assert_ne!(location_key(&new_babbage), location_key(&lorville));
    }

    #[test]
    fn location_key_shape_is_system_pipe_planet_pipe_city() {
        // Wire-stable encoding — the walker treats this as opaque, but
        // pinning the shape catches accidental reorders that would
        // silently break run matching.
        let r = resolve(
            "location_inventory_requested",
            &json!({"location": "Stanton2_Orison"}),
            ts(),
            None,
        )
        .unwrap();
        assert_eq!(location_key(&r), "Stanton|Crusader|Orison");
    }

    #[test]
    fn location_key_empty_segments_for_in_transit() {
        let r = resolve(
            "location_inventory_requested",
            &json!({"location": "INVALID_LOCATION_ID"}),
            ts(),
            None,
        )
        .unwrap();
        // All three resolution fields are None — key collapses to the
        // empty triple. Two in-transit readings share the same key, so
        // the run walker treats consecutive "in transit" events as one
        // dwell, which matches the intent for the chip.
        assert_eq!(location_key(&r), "||");
    }

    #[test]
    fn shard_hint_carried_through_to_planet_terrain_resolution() {
        let payload = json!({"planet": "OOC_Stanton_1_Hurston"});
        let r = resolve(
            "planet_terrain_load",
            &payload,
            ts(),
            Some("pub_euw1b_test".to_string()),
        )
        .unwrap();
        assert_eq!(r.shard.as_deref(), Some("pub_euw1b_test"));
        // The planet attribution still wins for the headline; shard
        // is contextual extra info.
        assert_eq!(r.planet.as_deref(), Some("Hurston"));
    }
}
