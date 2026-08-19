//! Tray-side location catalogue loader.
//!
//! The tray needs to classify game-log location strings without a
//! round-trip to the cloud — that's what makes "local-first
//! personal Star Citizen metrics" actually local-first. To support
//! offline first-launch (fresh install, no network), we ship a
//! small **bootstrap snapshot** baked into the binary via
//! `include_str!`. After the first successful
//! `GET /v1/reference/location` round-trip the tray persists a
//! larger snapshot to disk and uses that on subsequent launches.
//!
//! This module owns the in-memory `LocationCatalog` that the
//! classifier consumes. Three load paths, tried in order:
//!
//!   1. Persisted disk snapshot (most recent server fetch).
//!   2. Bundled bootstrap (`assets/location_catalog.bootstrap.json`).
//!   3. Empty catalog (graceful degradation — the classifier still
//!      runs synthetic+heuristic+fallback paths).
//!
//! See `docs/PLAN-LOCATION-TAXONOMY-V2.md` Phase 4 for the
//! cross-stack design.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use starstats_core::location_catalog::{LocationCatalog, LocationCatalogEntry};
use starstats_core::location_taxonomy::{LocationTaxonomy, LocationTier};

/// Bootstrap snapshot baked into the binary. ~15 hero locations —
/// enough that the classifier returns *something* on a fresh
/// install before the first server fetch. Production users get
/// the full ~1955-row catalogue at first sync.
const BOOTSTRAP_JSON: &str = include_str!("../assets/location_catalog.bootstrap.json");

/// Wire shape of the bundled JSON. Lives here (not in `starstats-core`)
/// because it's a tray-specific persistence detail — the server
/// neither produces nor consumes this file format.
#[derive(Debug, Deserialize)]
struct CatalogSnapshot {
    #[serde(default)]
    entries: Vec<LocationCatalogEntry>,
}

/// Load the bundled bootstrap catalog. Panics on parse error —
/// the JSON is shipped with the binary, so a bad payload here is a
/// build-time bug, not a runtime concern.
pub fn load_bootstrap() -> LocationCatalog {
    let snap: CatalogSnapshot =
        serde_json::from_str(BOOTSTRAP_JSON).expect("bundled bootstrap JSON must parse");
    LocationCatalog::from_entries(snap.entries)
}

/// Load a snapshot from a JSON file at the given path. Returns an
/// empty catalog (logged as a warning) on any failure — caller
/// falls back to `load_bootstrap()`.
pub fn load_from_path(path: &Path) -> LocationCatalog {
    match std::fs::read_to_string(path) {
        Ok(body) => match serde_json::from_str::<CatalogSnapshot>(&body) {
            Ok(snap) => LocationCatalog::from_entries(snap.entries),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "location catalog snapshot at {} failed to parse; using empty",
                    path.display()
                );
                LocationCatalog::default()
            }
        },
        Err(e) => {
            tracing::debug!(
                error = %e,
                path = %path.display(),
                "no persisted location catalog snapshot at {}; will use bootstrap",
                path.display()
            );
            LocationCatalog::default()
        }
    }
}

/// Resolve a catalog: prefer the persisted snapshot, fall back to
/// the bundled bootstrap, fall back to empty. Caller passes the
/// expected snapshot path (typically `<appdata>/location_catalog.json`).
pub fn resolve_catalog(snapshot_path: Option<&Path>) -> LocationCatalog {
    if let Some(path) = snapshot_path {
        let from_disk = load_from_path(path);
        if !from_disk.is_empty() {
            tracing::info!(
                entries = from_disk.len(),
                path = %path.display(),
                "loaded location catalog from disk snapshot"
            );
            return from_disk;
        }
    }
    let bootstrap = load_bootstrap();
    tracing::info!(
        entries = bootstrap.len(),
        "loaded location catalog from bundled bootstrap"
    );
    bootstrap
}

/// Serialise a catalog to disk JSON. Called after a successful
/// `/v1/reference/location` fetch so the next launch gets the
/// fresher data without re-downloading. Returns the byte count
/// written.
pub fn persist_to_path(entries: &[LocationCatalogEntry], path: &Path) -> std::io::Result<usize> {
    let snap = serde_json::json!({
        "version": 1,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "entries": entries,
    });
    let body = serde_json::to_string(&snap)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &body)?;
    Ok(body.len())
}

// ---- server refresh -------------------------------------------------

/// One entry from the `/v1/reference/location` listing. Lenient — only
/// the fields the classifier needs are deserialised; the server schema
/// can grow without breaking the client (mirrors `ReferenceListingResponse`
/// in `commands.rs`).
#[derive(Debug, Deserialize)]
struct ServerLocationEntry {
    class_name: Option<String>,
    display_name: Option<String>,
    slug: Option<String>,
    #[serde(default)]
    summary: ServerLocationSummary,
}

#[derive(Debug, Default, Deserialize)]
struct ServerLocationSummary {
    system: Option<String>,
    parent: Option<String>,
    tag: Option<String>,
    classification: Option<String>,
    tier: Option<LocationTier>,
    subtype: Option<String>,
}

/// Map a server listing entry to a catalogue entry. Drops entries
/// without a display name or any usable key — they can't be indexed.
/// Locations have no upstream `class_name`, so it falls back to `slug`
/// (the server does the same).
fn map_server_entry(e: ServerLocationEntry) -> Option<LocationCatalogEntry> {
    let display_name = e.display_name.filter(|s| !s.trim().is_empty())?;
    let slug = e.slug.unwrap_or_default();
    let class_name = e
        .class_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| slug.clone());
    if slug.is_empty() && class_name.is_empty() {
        return None;
    }
    Some(LocationCatalogEntry {
        slug,
        display_name,
        class_name,
        system: e.summary.system,
        parent_body: e.summary.parent,
        engine_tag: e.summary.tag,
        classification: e.summary.classification,
        taxonomy: LocationTaxonomy {
            tier: e.summary.tier,
            subtype: e.summary.subtype,
            ..LocationTaxonomy::default()
        },
    })
}

/// Fetch the full location catalogue from the paired server, map it to
/// catalogue entries, and persist the snapshot to `snapshot_path` for
/// the next launch. Returns the parsed entries so the caller can
/// hot-swap the in-memory catalogue. Best-effort — a network/parse
/// failure leaves the existing snapshot/bootstrap in place.
pub async fn fetch_and_persist(
    api_url: &str,
    snapshot_path: &Path,
) -> Result<Vec<LocationCatalogEntry>, String> {
    let api_url = api_url.trim().trim_end_matches('/');
    if api_url.is_empty() {
        return Err("no api_url configured".to_string());
    }
    let url = format!("{api_url}/v1/reference/location");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("contact api: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("server returned {}", resp.status()));
    }

    #[derive(Default, Deserialize)]
    struct Listing {
        #[serde(default)]
        entries: Vec<ServerLocationEntry>,
    }
    let body: Listing = resp
        .json()
        .await
        .map_err(|e| format!("parse response: {e}"))?;
    let entries: Vec<LocationCatalogEntry> = body
        .entries
        .into_iter()
        .filter_map(map_server_entry)
        .collect();
    if entries.is_empty() {
        return Err("server returned no usable location entries".to_string());
    }
    persist_to_path(&entries, snapshot_path).map_err(|e| format!("persist snapshot: {e}"))?;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_server_entry_builds_catalog_entry() {
        let e = ServerLocationEntry {
            class_name: Some("Stanton2b_Daymar".into()),
            display_name: Some("Daymar".into()),
            slug: Some("daymar".into()),
            summary: ServerLocationSummary {
                system: Some("Stanton".into()),
                parent: Some("Crusader".into()),
                tag: Some("Stanton2b".into()),
                classification: Some("Moon".into()),
                tier: Some(LocationTier::AstronomicalObject),
                subtype: Some("moon".into()),
            },
        };
        let c = map_server_entry(e).expect("maps");
        assert_eq!(c.slug, "daymar");
        assert_eq!(c.display_name, "Daymar");
        assert_eq!(c.engine_tag.as_deref(), Some("Stanton2b"));
        assert_eq!(c.system.as_deref(), Some("Stanton"));
        assert_eq!(c.taxonomy.tier, Some(LocationTier::AstronomicalObject));
        assert!(c.is_present());
    }

    #[test]
    fn map_server_entry_falls_back_class_name_to_slug() {
        let e = ServerLocationEntry {
            class_name: None,
            display_name: Some("Aberdeen".into()),
            slug: Some("aberdeen".into()),
            summary: ServerLocationSummary::default(),
        };
        assert_eq!(map_server_entry(e).expect("maps").class_name, "aberdeen");
    }

    #[test]
    fn map_server_entry_drops_entry_without_display_name() {
        let e = ServerLocationEntry {
            class_name: Some("x".into()),
            display_name: None,
            slug: Some("x".into()),
            summary: ServerLocationSummary::default(),
        };
        assert!(map_server_entry(e).is_none());
    }
    use starstats_core::location_taxonomy::{LocationTier, Placement};

    #[test]
    fn bootstrap_json_parses_and_indexes() {
        let cat = load_bootstrap();
        assert!(
            cat.len() >= 10,
            "bootstrap should ship at least 10 hero locations, got {}",
            cat.len()
        );
    }

    #[test]
    fn bootstrap_contains_lorville_with_enrichment() {
        let cat = load_bootstrap();
        let hit = cat.lookup_by_slug("lorville").expect("lorville present");
        assert_eq!(hit.display_name, "Lorville");
        assert_eq!(hit.system.as_deref(), Some("Stanton"));
        assert_eq!(hit.taxonomy.tier, Some(LocationTier::LandingZone));
        assert_eq!(hit.taxonomy.subtype.as_deref(), Some("city"));
        assert_eq!(
            hit.taxonomy.placement,
            Some(Placement::OnBody {
                body: "Hurston".into()
            })
        );
    }

    #[test]
    fn bootstrap_indexes_engine_tag_for_aberdeen() {
        let cat = load_bootstrap();
        let hit = cat.lookup_by_engine_tag("Stanton1b").expect("Stanton1b");
        assert_eq!(hit.display_name, "Aberdeen");
        assert_eq!(hit.taxonomy.subtype.as_deref(), Some("moon"));
    }

    #[test]
    fn resolve_catalog_falls_back_to_bootstrap_when_disk_missing() {
        let nowhere = std::path::PathBuf::from("definitely-does-not-exist-on-disk-2026-05-22.json");
        let cat = resolve_catalog(Some(&nowhere));
        assert!(cat.len() >= 10, "expected bootstrap fallback");
    }

    #[test]
    fn persist_and_reload_round_trip() {
        let tmp = std::env::temp_dir().join(format!(
            "starstats-catalog-test-{}.json",
            std::process::id()
        ));
        let bootstrap = load_bootstrap();
        let entries: Vec<_> = bootstrap.iter().cloned().collect();
        let n = persist_to_path(&entries, &tmp).expect("persist");
        assert!(n > 0);

        let loaded = load_from_path(&tmp);
        assert_eq!(loaded.len(), bootstrap.len());
        // Spot-check a known entry survived the round trip.
        assert!(loaded.lookup_by_slug("lorville").is_some());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_from_path_returns_empty_on_garbage_json() {
        let tmp =
            std::env::temp_dir().join(format!("starstats-catalog-bad-{}.json", std::process::id()));
        std::fs::write(&tmp, "not json at all").unwrap();
        let cat = load_from_path(&tmp);
        assert!(cat.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }
}
