//! Per-category numeric vectors for the multi-ship comparison endpoint.
//!
//! For each entry we extract its numeric metadata leaves (reusing
//! `starstats_core::stats::numeric_leaves`) into a `slug -> CompareEntry`
//! map, cached in memory and primed by the reconcile cron — the same
//! lifecycle as `ReferenceStatsCache`. The `/compare` route filters this
//! map to the requested slugs, so a lookup is O(requested) with no DB hit.

use crate::reference_data::ReferenceCategory;
use crate::reference_store::{ReferenceStore, ReferenceStoreError};
use serde::{Deserialize, Serialize};
use starstats_core::peer_group::peer_group;
use starstats_core::stats::numeric_leaves;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// One ship's numeric profile for comparison.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CompareEntry {
    pub slug: String,
    pub class_name: String,
    pub display_name: String,
    pub peer_group: String,
    #[schema(value_type = std::collections::HashMap<String, f64>)]
    pub metrics: HashMap<String, f64>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CompareResponse {
    pub entries: Vec<CompareEntry>,
}

/// A cached ship's vector plus the cohort keys it belongs to (for the
/// `/cohort` bulk-add filter). Cohort keys are computed once at cache
/// build from full metadata (the CompareEntry only carries numeric
/// metrics + peer_group).
#[derive(Debug, Clone)]
pub struct VectorRecord {
    pub entry: CompareEntry,
    /// Read by the `/cohort` handler via `members_for_cohort`.
    pub cohort_keys: Vec<String>,
}

/// Build the `slug -> VectorRecord` map for a category. Entries without
/// a slug are skipped (they can't be addressed by the slug-keyed route).
pub fn build_vectors<I>(category: ReferenceCategory, entries: I) -> HashMap<String, VectorRecord>
where
    I: IntoIterator<Item = crate::reference_data::ReferenceEntry>,
{
    let mut out = HashMap::new();
    for e in entries {
        let Some(slug) = e.slug.clone() else { continue };
        let metrics = numeric_leaves(&e.metadata).into_iter().collect();
        let cohort_keys = starstats_core::cohort::cohort_keys(category.as_str(), &e.metadata);
        let pg = peer_group(category.as_str(), &e.metadata);
        out.insert(
            slug.clone(),
            VectorRecord {
                entry: CompareEntry {
                    slug,
                    class_name: e.class_name,
                    display_name: e.display_name,
                    peer_group: pg,
                    metrics,
                },
                cohort_keys,
            },
        );
    }
    out
}

/// Entries whose cohort membership includes `key`. Bounded to keep the
/// bulk-add response small. Sorted deterministically (display_name then slug)
/// so the response is stable across HashMap iteration orders.
pub fn members_for_cohort(map: &HashMap<String, VectorRecord>, key: &str) -> Vec<CompareEntry> {
    const MAX_MEMBERS: usize = 60;
    let mut members: Vec<CompareEntry> = map
        .values()
        .filter(|r| r.cohort_keys.iter().any(|k| k == key))
        .map(|r| r.entry.clone())
        .collect();
    members.sort_by(|a, b| {
        a.display_name
            .cmp(&b.display_name)
            .then(a.slug.cmp(&b.slug))
    });
    members.truncate(MAX_MEMBERS);
    members
}

struct CachedVectors {
    built_at: Instant,
    map: Arc<HashMap<String, VectorRecord>>,
}

/// In-memory cache of per-category vector maps. Mirrors
/// `ReferenceStatsCache` (TTL serve + cron-primed rebuild).
pub struct ReferenceVectorsCache {
    ttl: Duration,
    inner: RwLock<HashMap<&'static str, CachedVectors>>,
}

impl ReferenceVectorsCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Serve the category's vector map, rebuilding when missing/stale.
    pub async fn serve<R: ReferenceStore>(
        &self,
        cat: ReferenceCategory,
        store: &R,
    ) -> Result<Arc<HashMap<String, VectorRecord>>, ReferenceStoreError> {
        {
            let guard = self.inner.read().await;
            if let Some(c) = guard.get(cat.as_str()) {
                if c.built_at.elapsed() < self.ttl {
                    return Ok(c.map.clone());
                }
            }
        }
        self.rebuild(cat, store).await
    }

    /// Rebuild + store the category's vector map (also cron-called).
    pub async fn rebuild<R: ReferenceStore>(
        &self,
        cat: ReferenceCategory,
        store: &R,
    ) -> Result<Arc<HashMap<String, VectorRecord>>, ReferenceStoreError> {
        let entries = store.list_category(cat).await?;
        let map = Arc::new(build_vectors(cat, entries));
        let mut guard = self.inner.write().await;
        guard.insert(
            cat.as_str(),
            CachedVectors {
                built_at: Instant::now(),
                map: map.clone(),
            },
        );
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_data::{ReferenceCategory, ReferenceEntry};
    use serde_json::json;

    fn entry(slug: Option<&str>, role: &str, scm: i64) -> ReferenceEntry {
        ReferenceEntry {
            category: ReferenceCategory::Vehicle,
            class_name: format!("CL_{slug:?}"),
            display_name: format!("Ship {scm}"),
            slug: slug.map(str::to_owned),
            metadata: json!({ "role": role, "speed": { "scm": scm }, "health": 1000 }),
        }
    }

    fn json_entry(
        slug: &str,
        role: &str,
        make: &str,
        cargo: i64,
    ) -> crate::reference_data::ReferenceEntry {
        crate::reference_data::ReferenceEntry {
            category: ReferenceCategory::Vehicle,
            class_name: slug.to_uppercase(),
            display_name: format!("Ship {slug}"),
            slug: Some(slug.into()),
            metadata: serde_json::json!({ "role": role, "manufacturer": make, "cargo_capacity": cargo, "speed": { "scm": 200 } }),
        }
    }

    #[test]
    fn builds_slug_keyed_vectors_and_skips_slugless() {
        let map = build_vectors(
            ReferenceCategory::Vehicle,
            vec![
                entry(Some("a"), "Light Fighter", 220),
                entry(None, "Light Fighter", 999), // skipped (no slug)
            ],
        );
        assert_eq!(map.len(), 1);
        let a = &map.get("a").expect("slug a present").entry;
        assert_eq!(a.peer_group, "combat");
        assert_eq!(a.metrics.get("speed.scm").copied(), Some(220.0));
        assert_eq!(a.metrics.get("health").copied(), Some(1000.0));
    }

    #[test]
    fn members_for_cohort_filters_cached_vectors() {
        let entries = vec![
            json_entry("a", "Interceptor", "Aegis Dynamics", 0),
            json_entry("b", "Interceptor", "Aegis Dynamics", 0),
            json_entry("c", "Cargo", "Drake Interplanetary", 500),
        ];
        let map = build_vectors(ReferenceCategory::Vehicle, entries);
        let combat = members_for_cohort(&map, "type:interceptor");
        let mut slugs: Vec<&str> = combat.iter().map(|e| e.slug.as_str()).collect();
        slugs.sort();
        assert_eq!(slugs, vec!["a", "b"]);
        assert!(members_for_cohort(&map, "type:nope").is_empty());
    }

    #[test]
    fn members_for_cohort_caps_at_60() {
        let entries: Vec<_> = (0..70)
            .map(|i| json_entry(&format!("s{i:03}"), "Interceptor", "Aegis Dynamics", 0))
            .collect();
        let map = build_vectors(ReferenceCategory::Vehicle, entries);
        assert_eq!(members_for_cohort(&map, "type:interceptor").len(), 60);
    }
}
