//! In-memory snapshot of the Star Citizen location catalogue, kept
//! warm between cron refreshes for the ingest classifier and any
//! ad-hoc consumers.
//!
//! Why a cache: building the catalog from `reference_registry`
//! requires scanning ~1955 rows + deserializing the per-row
//! `metadata.taxonomy_v2` blob — too expensive to do per request.
//! The cache holds the latest snapshot as an `Arc<LocationCatalog>`
//! so consumers get O(1) clone-equivalent reads.
//!
//! Lifecycle:
//!   * **Startup** — `LocationCatalogCache::load_from_store(store)`
//!     populates the initial snapshot. A startup with an empty
//!     reference_registry (fresh deploy, primary cron hasn't run yet)
//!     produces an empty catalog; the ingest classifier degrades to
//!     synthetic+heuristic+fallback paths cleanly.
//!   * **Refresh** — both the primary reference cron AND the
//!     enrichment cron in `main.rs` call `refresh(store)` after
//!     a successful upsert. Failures log + retain the previous
//!     snapshot (stale data > no data).
//!
//! Concurrency: snapshot reads via `snapshot()` clone an `Arc`
//! (cheap), so concurrent ingest tasks never block each other.
//! Writes via `refresh()` use a brief write lock and replace the
//! inner Arc atomically — readers see either the old snapshot or
//! the new one, never a torn read.

use std::sync::Arc;

use starstats_core::location_catalog::{LocationCatalog, LocationCatalogEntry};
use starstats_core::location_taxonomy::LocationTaxonomy;
use tokio::sync::RwLock;

use crate::reference_data::{ReferenceCategory, ReferenceEntry};
use crate::reference_store::{ReferenceStore, ReferenceStoreError};

#[derive(Debug, Default, Clone)]
pub struct LocationCatalogCache {
    inner: Arc<RwLock<Arc<LocationCatalog>>>,
}

impl LocationCatalogCache {
    /// Empty cache — useful in tests and as a bootstrap value before
    /// the first `refresh` call lands.
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(LocationCatalog::default()))),
        }
    }

    /// Build a cache pre-seeded with a fixed catalog. Test-only — lets
    /// route-layer unit tests exercise catalog-dependent classification
    /// (hierarchy backfill, friendly names) without a `ReferenceStore`.
    #[cfg(test)]
    pub fn from_catalog_for_test(catalog: LocationCatalog) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(catalog))),
        }
    }

    /// Build an initial snapshot from the store. Equivalent to
    /// `empty()` followed by `refresh()`; provided as a convenience
    /// for the startup path in `main.rs`.
    pub async fn load_from_store<R: ReferenceStore + ?Sized>(
        store: &R,
    ) -> Result<Self, ReferenceStoreError> {
        let cache = Self::empty();
        cache.refresh(store).await?;
        Ok(cache)
    }

    /// Get the current snapshot. Clones an Arc — cheap.
    #[allow(dead_code)] // Consumed by upcoming ingest classifier hook.
    pub async fn snapshot(&self) -> Arc<LocationCatalog> {
        Arc::clone(&*self.inner.read().await)
    }

    /// Synchronous variant for callers in non-async contexts
    /// (`blocking_read` panics if a write lock is held — only used
    /// from a context where that's impossible).
    #[allow(dead_code)] // Consumed by upcoming ingest classifier hook.
    pub fn blocking_snapshot(&self) -> Arc<LocationCatalog> {
        Arc::clone(&*self.inner.blocking_read())
    }

    /// Rebuild the catalog from the store and atomically swap in the
    /// new snapshot. Callers are the cron tasks in `main.rs`.
    pub async fn refresh<R: ReferenceStore + ?Sized>(
        &self,
        store: &R,
    ) -> Result<usize, ReferenceStoreError> {
        let entries = store.list_category(ReferenceCategory::Location).await?;
        let catalog = LocationCatalog::from_entries(
            entries.into_iter().map(entry_to_catalog_entry).collect(),
        );
        let len = catalog.len();
        let mut guard = self.inner.write().await;
        *guard = Arc::new(catalog);
        Ok(len)
    }
}

/// Translate a generic `ReferenceEntry` (the storage shape) into a
/// `LocationCatalogEntry` (the classifier-facing shape). Pulls the
/// hierarchy fields out of the wiki-supplied `metadata` plus the
/// taxonomy enrichment blob mirrored INTO metadata.taxonomy_v2 by
/// `ReferenceStore::apply_location_taxonomies`.
pub fn entry_to_catalog_entry(e: ReferenceEntry) -> LocationCatalogEntry {
    let meta = e.metadata.as_object().cloned().unwrap_or_default();

    let nested_str = |obj_key: &str, key: &str| -> Option<String> {
        meta.get(obj_key)?
            .get(key)?
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };

    let taxonomy: LocationTaxonomy = meta
        .get("taxonomy_v2")
        .and_then(|v| serde_json::from_value::<LocationTaxonomy>(v.clone()).ok())
        .unwrap_or_default();

    LocationCatalogEntry {
        slug: e.slug.unwrap_or_default(),
        display_name: e.display_name,
        class_name: e.class_name,
        system: nested_str("star", "name"),
        parent_body: nested_str("parent", "name"),
        engine_tag: nested_str("tag", "name"),
        classification: nested_str("type", "classification"),
        taxonomy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_store::test_support::MemoryReferenceStore;
    use starstats_core::location_taxonomy::LocationTier;

    fn lorville_row() -> ReferenceEntry {
        ReferenceEntry {
            category: ReferenceCategory::Location,
            class_name: "Lorville".to_string(),
            display_name: "Lorville".to_string(),
            slug: Some("lorville".to_string()),
            metadata: serde_json::json!({
                "star":   { "name": "Stanton" },
                "parent": { "name": "Hurston" },
                "tag":    { "name": "Lorville" },
                "type":   { "classification": "Settlement" },
                "taxonomy_v2": {
                    "tier":    "landing_zone",
                    "subtype": "city",
                    "placement": { "kind": "on_body", "body": "Hurston" }
                }
            }),
        }
    }

    #[test]
    fn entry_to_catalog_entry_pulls_every_known_field() {
        let e = entry_to_catalog_entry(lorville_row());
        assert_eq!(e.slug, "lorville");
        assert_eq!(e.display_name, "Lorville");
        assert_eq!(e.class_name, "Lorville");
        assert_eq!(e.system.as_deref(), Some("Stanton"));
        assert_eq!(e.parent_body.as_deref(), Some("Hurston"));
        assert_eq!(e.engine_tag.as_deref(), Some("Lorville"));
        assert_eq!(e.classification.as_deref(), Some("Settlement"));
        assert_eq!(e.taxonomy.tier, Some(LocationTier::LandingZone));
        assert_eq!(e.taxonomy.subtype.as_deref(), Some("city"));
    }

    #[test]
    fn entry_to_catalog_entry_tolerates_missing_taxonomy_v2() {
        // Pre-enrichment row — metadata has no taxonomy_v2.
        let mut row = lorville_row();
        if let Some(obj) = row.metadata.as_object_mut() {
            obj.remove("taxonomy_v2");
        }
        let e = entry_to_catalog_entry(row);
        assert!(e.taxonomy.tier.is_none());
        assert!(e.taxonomy.subtype.is_none());
        // Wave 1 fields still resolve.
        assert_eq!(e.system.as_deref(), Some("Stanton"));
    }

    #[test]
    fn entry_to_catalog_entry_tolerates_completely_empty_metadata() {
        let row = ReferenceEntry {
            category: ReferenceCategory::Location,
            class_name: "Mystery".to_string(),
            display_name: "Mystery".to_string(),
            slug: Some("mystery".to_string()),
            metadata: serde_json::Value::Object(Default::default()),
        };
        let e = entry_to_catalog_entry(row);
        assert_eq!(e.slug, "mystery");
        assert!(e.system.is_none());
        assert!(e.taxonomy.tier.is_none());
    }

    #[tokio::test]
    async fn cache_load_and_snapshot_round_trip() {
        let store = MemoryReferenceStore::new();
        store.upsert_entries(&[lorville_row()]).await.expect("seed");

        let cache = LocationCatalogCache::load_from_store(&store)
            .await
            .expect("load");
        let snap = cache.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert!(snap.lookup_by_slug("lorville").is_some());
    }

    #[tokio::test]
    async fn cache_refresh_swaps_snapshot_atomically() {
        let store = MemoryReferenceStore::new();
        let cache = LocationCatalogCache::empty();

        // Initial refresh against empty store.
        cache.refresh(&store).await.expect("refresh");
        assert_eq!(cache.snapshot().await.len(), 0);

        // Seed + re-refresh.
        store.upsert_entries(&[lorville_row()]).await.expect("seed");
        let n = cache.refresh(&store).await.expect("refresh again");
        assert_eq!(n, 1);
        assert_eq!(cache.snapshot().await.len(), 1);
    }
}
