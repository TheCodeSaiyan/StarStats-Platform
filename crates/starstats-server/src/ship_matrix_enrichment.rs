//! RSI Ship Matrix enrichment — the first [`crate::enrichment::EnrichmentSource`].
//!
//! Joins RSI's official Ship Matrix (`/ship-matrix/index`) onto the
//! existing `reference_registry` **vehicle** rows, painting structured
//! specs (dimensions, speeds, crew, cargo, production status), the lore
//! description, and official media URLs into `metadata.ship_matrix`.
//!
//! ## Why a fuzzy name match
//!
//! Unlike `api.star-citizen.wiki` (which keys on the engine
//! `class_name`), the Ship Matrix has NO engine identifier — it keys on
//! the marketing `name` + a numeric `id`/`chassis_id`. So this source
//! can only ride ON TOP of the wiki's `class_name ↔ display_name`
//! mapping: it matches RSI `name` against our rows' `display_name`
//! (normalized), with a base-chassis prefix fallback. A prototype matched
//! ~91% of vehicles (73% exact name, +18% chassis prefix); the unmatched
//! remainder (odd variant names) are logged verbatim for a future alias
//! map — never silently dropped.
//!
//! ## ToS / media
//!
//! See `the release design notes`.
//! Specs are facts (safe); description + images are surfaced under the
//! ecosystem-norm posture with a comply-on-request kill-switch
//! (`STARSTATS_SHIP_MATRIX_MEDIA`) gating the media route.
//!
//! ## Failure semantics
//!
//! Mirrors `location_enrichment`: any fetch/parse failure collapses to
//! [`crate::enrichment::EnrichmentOutcome::UpstreamUnavailable`]; the
//! generic runner retains whatever enrichment is already in the store.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::enrichment::{EnrichmentOutcome, EnrichmentSource};
use crate::reference_data::{ReferenceCategory, ReferenceEntry};

/// Per-request HTTP timeout. The Ship Matrix index is a single ~5 MB
/// response; 20s is generous headroom over a normal ~2s fetch.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Body cap. The full Ship Matrix runs ~5 MB; 16 MB is ~3x headroom and
/// matches the per-page ceiling the wiki client uses for vehicles.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// The Ship Matrix index rejects the default reqwest User-Agent (and
/// non-browser agents generally), unlike `api.star-citizen.wiki`. A
/// browser UA is required to get a 200.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

const SHIP_MATRIX_URL: &str = "https://robertsspaceindustries.com/ship-matrix/index";

/// The metadata namespace this source writes under.
pub const NAMESPACE: &str = "ship_matrix";

/// Max image URLs kept per ship — the gallery doesn't need dozens.
const MAX_MEDIA: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum ShipMatrixError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("JSON parse error: {0}")]
    ParseJson(String),
    #[error("ship matrix reported success=false")]
    Unsuccessful,
}

/// Production [`EnrichmentSource`] backed by `reqwest`.
pub struct ShipMatrixSource {
    inner: reqwest::Client,
    endpoint: String,
}

impl ShipMatrixSource {
    /// Construct against the production Ship Matrix endpoint.
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::with_endpoint(SHIP_MATRIX_URL.to_string())
    }

    /// Construct against an arbitrary endpoint (tests / mocks).
    pub fn with_endpoint(endpoint: String) -> Result<Self, reqwest::Error> {
        let inner = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(BROWSER_USER_AGENT)
            // The endpoint is a fixed constant, but pin redirects off as
            // defence-in-depth so a surprise 30x can't steer the fetch.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { inner, endpoint })
    }

    async fn fetch_body(&self) -> Result<String, ShipMatrixError> {
        let resp = self
            .inner
            .get(&self.endpoint)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| ShipMatrixError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ShipMatrixError::Http(format!("HTTP {status}")));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ShipMatrixError::Http(e.to_string()))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(ShipMatrixError::Http(format!(
                "response body {} bytes exceeds cap {}",
                bytes.len(),
                MAX_RESPONSE_BYTES
            )));
        }
        String::from_utf8(bytes.to_vec()).map_err(|e| ShipMatrixError::ParseJson(e.to_string()))
    }
}

#[async_trait]
impl EnrichmentSource for ShipMatrixSource {
    fn category(&self) -> ReferenceCategory {
        ReferenceCategory::Vehicle
    }

    fn namespace(&self) -> &'static str {
        NAMESPACE
    }

    fn name(&self) -> &'static str {
        // Log label happens to equal the metadata key for this source;
        // reuse the const so a rename can't silently desync them.
        NAMESPACE
    }

    async fn fetch_and_match(&self, existing: &[ReferenceEntry]) -> EnrichmentOutcome {
        let body = match self.fetch_body().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "ship matrix: fetch failed");
                return EnrichmentOutcome::UpstreamUnavailable;
            }
        };

        let ships = match parse_ship_matrix_body(&body) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "ship matrix: parse failed");
                return EnrichmentOutcome::UpstreamUnavailable;
            }
        };

        if ships.is_empty() {
            tracing::warn!("ship matrix: zero ships parsed (upstream empty?)");
            return EnrichmentOutcome::UpstreamUnavailable;
        }

        let matched_at = chrono::Utc::now().to_rfc3339();
        let (pairs, unmatched) = match_ships(existing, &ships, &matched_at);

        // No silent caps: log the count AND the verbatim unmatched
        // display names so the ~9% gap is actionable for an alias map.
        tracing::info!(
            ships_seen = ships.len(),
            rows = existing.len(),
            matched = pairs.len(),
            unmatched = unmatched.len(),
            "ship matrix: match complete"
        );
        if !unmatched.is_empty() {
            tracing::info!(unmatched_vehicles = ?unmatched, "ship matrix: unmatched vehicle rows");
        }

        EnrichmentOutcome::Entries(pairs)
    }
}

// ---- pure parsing + matching (unit-tested without a network call) ---

/// Parse the Ship Matrix `{ success, data: [...] }` envelope into the
/// raw ship `Value`s. Returns [`ShipMatrixError::Unsuccessful`] when the
/// `success` flag is falsy (so a maintenance page that still returns 200
/// doesn't look like real data).
pub fn parse_ship_matrix_body(body: &str) -> Result<Vec<Value>, ShipMatrixError> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| ShipMatrixError::ParseJson(e.to_string()))?;

    // `success` is `1`/`0` (number) on the real endpoint; tolerate bool.
    let success = match root.get("success") {
        Some(Value::Number(n)) => n.as_i64().map(|v| v != 0).unwrap_or(false),
        Some(Value::Bool(b)) => *b,
        _ => false,
    };
    if !success {
        return Err(ShipMatrixError::Unsuccessful);
    }

    match root.get("data") {
        Some(Value::Array(items)) => Ok(items.clone()),
        _ => Ok(Vec::new()),
    }
}

/// Normalize a name for matching: ASCII-alphanumeric only, lowercased.
/// `"F8A Lightning"` → `"f8alightning"`, `"Aegis Avenger Stalker"` →
/// `"aegisavengerstalker"`.
fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Split a name into lowercased ASCII-alphanumeric word tokens on any
/// non-alphanumeric boundary. `"Avenger Titan Renegade"` →
/// `["avenger","titan","renegade"]`. Used by the chassis fallback so a
/// base chassis only matches a variant that extends it at a WORD
/// boundary — never an arbitrary leading substring.
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// True when `base` is a STRICT leading token-prefix of `full`: every
/// token of `base` matches the corresponding leading token of `full`,
/// and `full` carries at least one extra token (so it's a more specific
/// variant, not an equal/shorter name).
fn is_strict_token_prefix(base: &[String], full: &[String]) -> bool {
    !base.is_empty() && base.len() < full.len() && full[..base.len()] == *base
}

/// Match every existing vehicle row against the Ship Matrix ships.
/// Returns `(pairs, unmatched_display_names)` where `pairs` is
/// `(class_name, blob)` ready for `apply_enrichment`.
///
/// Strategy per row (`display_name`):
///  1. exact normalized-name match against a ship `name`;
///  2. base-chassis fallback — the ship with the MOST tokens whose token
///     sequence is a strict leading prefix of the row's tokens (the row
///     is a more specific variant of that base chassis). Matching on
///     word-boundary tokens (not raw concatenated substrings) avoids
///     false positives like "Mole" prefixing "Moles…", and the
///     strict-prefix-only rule drops the inverse over-match where a
///     one-word row ("Hull") would otherwise grab every "Hull A…E".
fn match_ships(
    existing: &[ReferenceEntry],
    ships: &[Value],
    matched_at: &str,
) -> (Vec<(String, Value)>, Vec<String>) {
    // Exact index by concatenated normalized name (first occurrence
    // wins); token index for the chassis-prefix fallback.
    let mut by_norm: HashMap<String, &Value> = HashMap::with_capacity(ships.len());
    let mut token_list: Vec<(Vec<String>, &Value)> = Vec::with_capacity(ships.len());
    for ship in ships {
        let Some(name) = ship.get("name").and_then(Value::as_str) else {
            continue;
        };
        let norm = normalize_name(name);
        if norm.is_empty() {
            continue;
        }
        by_norm.entry(norm).or_insert(ship);
        let toks = tokens(name);
        if !toks.is_empty() {
            token_list.push((toks, ship));
        }
    }

    let mut pairs = Vec::new();
    let mut unmatched = Vec::new();

    for row in existing {
        let row_norm = normalize_name(&row.display_name);
        if row_norm.is_empty() {
            unmatched.push(row.display_name.clone());
            continue;
        }

        if let Some(ship) = by_norm.get(&row_norm) {
            pairs.push((row.class_name.clone(), build_blob(ship, "name", matched_at)));
            continue;
        }

        // Chassis fallback: the ship whose tokens are a strict leading
        // prefix of the row's tokens (row = a more specific variant),
        // preferring the longest such base chassis.
        let row_tokens = tokens(&row.display_name);
        let best = token_list
            .iter()
            .filter(|(st, _)| is_strict_token_prefix(st, &row_tokens))
            .max_by_key(|(st, _)| st.len());

        match best {
            Some((_, ship)) => {
                pairs.push((
                    row.class_name.clone(),
                    build_blob(ship, "chassis", matched_at),
                ));
            }
            None => unmatched.push(row.display_name.clone()),
        }
    }

    (pairs, unmatched)
}

/// Build the `metadata.ship_matrix` blob from a raw ship `Value`.
/// Only present spec fields are included, so a sparse upstream record
/// yields a sparse (not zero-filled) blob.
fn build_blob(ship: &Value, matched_by: &str, matched_at: &str) -> Value {
    let mut specs = serde_json::Map::new();
    for (key, aliases) in [
        ("length", &["length"][..]),
        ("beam", &["beam"][..]),
        ("height", &["height"][..]),
        ("mass", &["mass"][..]),
        ("scm_speed", &["scm_speed"][..]),
        ("afterburner_speed", &["afterburner_speed"][..]),
        ("min_crew", &["min_crew"][..]),
        ("max_crew", &["max_crew"][..]),
        ("cargo", &["cargocapacity", "cargo"][..]),
    ] {
        if let Some(v) = num(ship, aliases) {
            specs.insert(key.to_string(), json!(v));
        }
    }

    let mut blob = serde_json::Map::new();
    blob.insert("specs".to_string(), Value::Object(specs));
    if let Some(s) = production_status(ship) {
        blob.insert("production_status".to_string(), json!(s));
    }
    if let Some(d) = ship
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        blob.insert("description".to_string(), json!(d));
    }
    let media = extract_media(ship);
    if !media.is_empty() {
        blob.insert("media".to_string(), json!(media));
    }
    blob.insert("matched_by".to_string(), json!(matched_by));
    blob.insert("matched_at".to_string(), json!(matched_at));

    Value::Object(blob)
}

/// Coerce a numeric-ish field to `f64`. The Ship Matrix returns specs as
/// strings (`"length": "22.50000000"`); tolerate plain numbers too.
fn num(ship: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        match ship.get(*k) {
            Some(Value::Number(n)) => return n.as_f64(),
            Some(Value::String(s)) => {
                let t = s.trim();
                if !t.is_empty() {
                    if let Ok(f) = t.parse::<f64>() {
                        return Some(f);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// `production_status` is a slug string on the real endpoint, but
/// tolerate an object carrying `name`/`slug`.
fn production_status(ship: &Value) -> Option<String> {
    match ship.get("production_status") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(Value::Object(o)) => o
            .get("name")
            .or_else(|| o.get("slug"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

/// Collect up to [`MAX_MEDIA`] image URLs from a ship's `media` array.
/// Defensive against the exact shape: takes `source_url` strings and any
/// http(s) string values nested under each entry's `images` object.
fn extract_media(ship: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    let Some(Value::Array(entries)) = ship.get("media") else {
        return urls;
    };
    for entry in entries {
        // ONE canonical URL per media entry. RSI's `images` object is
        // ~50 size/crop variants of the SAME picture, so iterating it
        // would fill the gallery with duplicates of one render. Prefer
        // `source_url` (the full image); fall back to a single large
        // store/hub variant only when `source_url` is absent.
        let url = entry
            .get("source_url")
            .and_then(Value::as_str)
            .filter(|u| !u.trim().is_empty())
            .or_else(|| pick_image_variant(entry));
        if let Some(u) = url {
            push_url(&mut urls, u);
        }
        if urls.len() >= MAX_MEDIA {
            break;
        }
    }
    urls.truncate(MAX_MEDIA);
    urls
}

/// Pick a single representative image-variant URL from a media entry's
/// `images` object, preferring larger renders. Only used when the entry
/// has no `source_url`.
fn pick_image_variant(entry: &Value) -> Option<&str> {
    let images = entry.get("images")?.as_object()?;
    for key in [
        "store_large",
        "store_hub_large",
        "hub_large",
        "slideshow",
        "banner",
        "cover",
    ] {
        if let Some(u) = images
            .get(key)
            .and_then(Value::as_str)
            .filter(|u| !u.trim().is_empty())
        {
            return Some(u);
        }
    }
    images
        .values()
        .find_map(|v| v.as_str().filter(|u| !u.trim().is_empty()))
}

fn push_url(urls: &mut Vec<String>, u: &str) {
    let u = u.trim();
    if (u.starts_with("https://") || u.starts_with("http://")) && !urls.iter().any(|e| e == u) {
        urls.push(u.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vehicle(class_name: &str, display_name: &str) -> ReferenceEntry {
        ReferenceEntry {
            category: ReferenceCategory::Vehicle,
            class_name: class_name.to_string(),
            display_name: display_name.to_string(),
            slug: Some(normalize_name(display_name)),
            metadata: json!({ "manufacturer": "Aegis Dynamics" }),
        }
    }

    /// Abridged real-shape Ship Matrix body: numeric specs as strings,
    /// production_status as a slug, media with source_url + images.
    fn sample_body() -> &'static str {
        r#"{
          "success": 1,
          "data": [
            {
              "id": 1,
              "chassis_id": 1,
              "name": "Aegis Avenger Stalker",
              "description": "A bounty hunter's ship.",
              "production_status": "flight-ready",
              "length": "22.50000000",
              "beam": "16.50000000",
              "height": "5.50000000",
              "mass": "57680.00000000",
              "scm_speed": "215",
              "afterburner_speed": "1210",
              "min_crew": "1",
              "max_crew": "1",
              "cargocapacity": "0",
              "media": [
                { "source_url": "https://media.rsi/avenger.jpg",
                  "images": { "store_small": "https://media.rsi/avenger_small.jpg" } }
              ]
            },
            {
              "id": 2,
              "name": "Avenger Titan",
              "production_status": "flight-ready",
              "length": "22.5",
              "cargocapacity": "8",
              "media": []
            }
          ]
        }"#
    }

    #[test]
    fn parse_extracts_ship_array() {
        let ships = parse_ship_matrix_body(sample_body()).expect("parse ok");
        assert_eq!(ships.len(), 2);
        assert_eq!(ships[0]["name"], "Aegis Avenger Stalker");
    }

    #[test]
    fn parse_rejects_unsuccessful() {
        let body = r#"{ "success": 0, "data": [] }"#;
        assert!(matches!(
            parse_ship_matrix_body(body),
            Err(ShipMatrixError::Unsuccessful)
        ));
    }

    #[test]
    fn parse_rejects_malformed_json() {
        assert!(matches!(
            parse_ship_matrix_body("not json"),
            Err(ShipMatrixError::ParseJson(_))
        ));
    }

    #[test]
    fn num_coerces_strings_and_numbers() {
        let v = json!({ "a": "22.5", "b": 8, "c": "", "d": "x" });
        assert_eq!(num(&v, &["a"]), Some(22.5));
        assert_eq!(num(&v, &["b"]), Some(8.0));
        assert_eq!(num(&v, &["c"]), None);
        assert_eq!(num(&v, &["d"]), None);
        assert_eq!(num(&v, &["missing", "b"]), Some(8.0));
    }

    #[test]
    fn build_blob_includes_only_present_specs_plus_media() {
        let ships = parse_ship_matrix_body(sample_body()).unwrap();
        let blob = build_blob(&ships[0], "name", "2026-06-12T00:00:00Z");
        assert_eq!(blob["specs"]["length"], 22.5);
        assert_eq!(blob["specs"]["max_crew"], 1.0);
        assert_eq!(blob["specs"]["cargo"], 0.0);
        assert_eq!(blob["production_status"], "flight-ready");
        assert_eq!(blob["description"], "A bounty hunter's ship.");
        assert_eq!(blob["matched_by"], "name");
        // ONE canonical URL per media entry: source_url wins, the
        // `images` size-variants are NOT pulled in (would be dupes).
        let media = blob["media"].as_array().unwrap();
        assert_eq!(
            media,
            &vec![serde_json::json!("https://media.rsi/avenger.jpg")]
        );
    }

    #[test]
    fn extract_media_one_url_per_entry_not_per_variant() {
        // Regression: one media entry with many `images` size-variants
        // of the SAME picture must yield exactly ONE gallery URL
        // (source_url), not a dozen duplicates.
        let ship = serde_json::json!({
            "media": [{
                "source_url": "https://media.rsi/a/source.jpg",
                "images": {
                    "store_large": "https://media.rsi/a/store_large.jpg",
                    "banner": "https://media.rsi/a/banner.jpg",
                    "heap_thumb": "https://media.rsi/a/heap_thumb.jpg"
                }
            }]
        });
        assert_eq!(extract_media(&ship), vec!["https://media.rsi/a/source.jpg"]);
    }

    #[test]
    fn extract_media_distinct_url_per_entry_and_variant_fallback() {
        // Two entries → two URLs. The second has no source_url, so it
        // falls back to a single (large, preferred) image variant.
        let ship = serde_json::json!({
            "media": [
                { "source_url": "https://media.rsi/a/source.jpg",
                  "images": { "store_large": "https://media.rsi/a/store_large.jpg" } },
                { "images": {
                    "heap_thumb": "https://media.rsi/b/heap_thumb.jpg",
                    "store_large": "https://media.rsi/b/store_large.jpg"
                } }
            ]
        });
        assert_eq!(
            extract_media(&ship),
            vec![
                "https://media.rsi/a/source.jpg",
                "https://media.rsi/b/store_large.jpg" // preferred variant, not heap_thumb
            ]
        );
    }

    #[test]
    fn build_blob_omits_media_key_when_empty() {
        let ships = parse_ship_matrix_body(sample_body()).unwrap();
        // Second ship has empty media.
        let blob = build_blob(&ships[1], "name", "2026-06-12T00:00:00Z");
        assert!(blob.get("media").is_none());
    }

    #[test]
    fn match_exact_name() {
        let existing = vec![vehicle("AEGS_Avenger_Stalker", "Aegis Avenger Stalker")];
        let ships = parse_ship_matrix_body(sample_body()).unwrap();
        let (pairs, unmatched) = match_ships(&existing, &ships, "t");
        assert_eq!(pairs.len(), 1);
        assert!(unmatched.is_empty());
        assert_eq!(pairs[0].0, "AEGS_Avenger_Stalker");
        assert_eq!(pairs[0].1["matched_by"], "name");
    }

    #[test]
    fn match_chassis_prefix_fallback() {
        // Our row "Avenger Titan Renegade" isn't in the matrix, but
        // "Avenger Titan" is a normalized prefix → chassis fallback.
        let existing = vec![vehicle(
            "AEGS_Avenger_Titan_Renegade",
            "Avenger Titan Renegade",
        )];
        let ships = parse_ship_matrix_body(sample_body()).unwrap();
        let (pairs, unmatched) = match_ships(&existing, &ships, "t");
        assert_eq!(pairs.len(), 1, "should fall back to chassis prefix");
        assert!(unmatched.is_empty());
        assert_eq!(pairs[0].1["matched_by"], "chassis");
    }

    #[test]
    fn match_records_unmatched_verbatim() {
        let existing = vec![vehicle("XIAN_Nox", "Nox")];
        let ships = parse_ship_matrix_body(sample_body()).unwrap();
        let (pairs, unmatched) = match_ships(&existing, &ships, "t");
        assert!(pairs.is_empty());
        assert_eq!(unmatched, vec!["Nox".to_string()]);
    }

    #[test]
    fn match_does_not_reverse_or_substring_overmatch() {
        // Regression guard for the chassis-fallback over-match: a row
        // that is a SHORTER name than a ship ("Avenger" vs ship "Avenger
        // Titan") must NOT reverse-match, and a row whose normalized
        // form is merely a leading substring of a ship token must not
        // either. Only strict token-prefix (row = variant of ship) wins.
        let existing = vec![
            // Shorter than any ship token sequence → no reverse match.
            vehicle("AEGS_Avenger_Base", "Avenger"),
            // "Aven" is a substring of "avenger" but not a whole token.
            vehicle("FAKE_Aven", "Aven"),
        ];
        let ships = parse_ship_matrix_body(sample_body()).unwrap();
        let (pairs, unmatched) = match_ships(&existing, &ships, "t");
        assert!(pairs.is_empty(), "neither row should chassis-match");
        assert_eq!(unmatched.len(), 2);
    }

    #[test]
    fn source_metadata_is_vehicle_ship_matrix() {
        let src = ShipMatrixSource::new().unwrap();
        assert_eq!(src.category(), ReferenceCategory::Vehicle);
        assert_eq!(src.namespace(), "ship_matrix");
    }
}
