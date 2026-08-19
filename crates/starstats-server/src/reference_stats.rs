//! Per-category peer-group stats over the cached reference data.
//!
//! For each category we group entries by `peer_group` (plus an
//! `__all__` whole-category bucket) and summarise every numeric metadata
//! leaf present in ≥ `MIN_SAMPLE` entries of that group as quantiles.
//! Mirrors `ReferenceListCache`: an in-memory cache of pre-serialized
//! JSON bytes, lazily rebuilt on a TTL and primed by the reconcile cron.

use crate::reference_data::ReferenceCategory;
use crate::reference_store::{ReferenceStore, ReferenceStoreError};
use axum::body::Bytes;
use serde::{Deserialize, Serialize};
use starstats_core::stats::{numeric_leaves, Quantiles};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A metric path needs at least this many samples in a peer group
/// before its quantiles are published — fewer than this and percentile
/// labels are noise.
const MIN_SAMPLE: usize = 5;

/// Whole-category fallback bucket key. Always emitted alongside the
/// per-`peer_group` buckets so a sparse group can fall back.
pub const ALL_BUCKET: &str = "__all__";

/// `peer_group -> metricPath -> Quantiles`.
pub type CategoryStats = HashMap<String, HashMap<String, Quantiles>>;

#[derive(Debug, Serialize, Deserialize)]
pub struct ReferenceStatsResponse {
    pub groups: CategoryStats,
}

/// Build the stats structure for one category from its entries' metadata.
pub fn build_category_stats<I>(category: ReferenceCategory, entries: I) -> CategoryStats
where
    I: IntoIterator<Item = serde_json::Value>,
{
    // group -> path -> raw values
    let mut acc: HashMap<String, HashMap<String, Vec<f64>>> = HashMap::new();
    for metadata in entries {
        let mut keys = starstats_core::cohort::cohort_keys(category.as_str(), &metadata);
        keys.push(ALL_BUCKET.to_string());
        let leaves = numeric_leaves(&metadata);
        for bucket in &keys {
            let g = acc.entry(bucket.clone()).or_default();
            for (path, val) in &leaves {
                g.entry(path.clone()).or_default().push(*val);
            }
        }
    }
    let mut out: CategoryStats = HashMap::new();
    for (group, paths) in acc {
        let mut metric_map: HashMap<String, Quantiles> = HashMap::new();
        for (path, vals) in paths {
            if vals.len() < MIN_SAMPLE {
                continue;
            }
            if let Some(q) = Quantiles::from_values(&vals) {
                metric_map.insert(path, q);
            }
        }
        if !metric_map.is_empty() {
            out.insert(group, metric_map);
        }
    }
    out
}

struct CachedStats {
    built_at: Instant,
    body: Bytes,
}

/// In-memory cache of serialized per-category stats responses, keyed by
/// category. Same shape + lifecycle as `ReferenceListCache`.
pub struct ReferenceStatsCache {
    ttl: Duration,
    inner: RwLock<HashMap<&'static str, CachedStats>>,
}

impl ReferenceStatsCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Serve cached bytes for `cat`, rebuilding when missing/stale.
    pub async fn serve<R: ReferenceStore>(
        &self,
        cat: ReferenceCategory,
        store: &R,
    ) -> Result<Bytes, ReferenceStoreError> {
        {
            let guard = self.inner.read().await;
            if let Some(c) = guard.get(cat.as_str()) {
                if c.built_at.elapsed() < self.ttl {
                    return Ok(c.body.clone());
                }
            }
        }
        self.rebuild(cat, store).await
    }

    /// Rebuild + store one category's stats. Also called by the reconcile
    /// cron after a successful category refresh.
    pub async fn rebuild<R: ReferenceStore>(
        &self,
        cat: ReferenceCategory,
        store: &R,
    ) -> Result<Bytes, ReferenceStoreError> {
        let entries = store.list_category(cat).await?;
        let metas = entries.into_iter().map(|e| e.metadata);
        let groups = build_category_stats(cat, metas);
        let json = serde_json::to_vec(&ReferenceStatsResponse { groups })
            .map_err(|e| ReferenceStoreError::Backend(e.to_string()))?;
        let body = Bytes::from(json);
        let mut guard = self.inner.write().await;
        guard.insert(
            cat.as_str(),
            CachedStats {
                built_at: Instant::now(),
                body: body.clone(),
            },
        );
        Ok(body)
    }
}

/// OpenAPI schema mirror of `starstats_core::stats::Quantiles`
/// (core has no utoipa dep). Keep field-for-field in sync.
// OpenAPI schema mirror only — the wire format uses starstats_core::stats::Quantiles directly.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct QuantilesSchema {
    pub min: f64,
    pub p10: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p90: f64,
    pub max: f64,
    pub n: usize,
}

/// OpenAPI schema mirror for the stats response: peer_group → (metricPath → Quantiles).
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReferenceStatsResponseSchema {
    #[schema(value_type = std::collections::HashMap<String, std::collections::HashMap<String, QuantilesSchema>>)]
    pub groups: CategoryStats,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fighters(n: usize) -> Vec<serde_json::Value> {
        (0..n)
            .map(|i| json!({ "role": "Light Fighter", "speed": { "scm": 200 + i as i64 } }))
            .collect()
    }

    #[test]
    fn builds_group_and_all_buckets_with_threshold() {
        let mut entries = fighters(6);
        // One support ship — its group has only 1 sample, so its bucket
        // is dropped, but it still contributes to __all__.
        entries.push(json!({ "role": "Medical", "speed": { "scm": 150 } }));

        let stats = build_category_stats(ReferenceCategory::Vehicle, entries);

        // family:combat bucket present (6 ≥ MIN_SAMPLE) with speed.scm.
        let combat = stats.get("family:combat").expect("family:combat bucket");
        assert!(combat.contains_key("speed.scm"));
        assert_eq!(combat["speed.scm"].n, 6);

        // family:support bucket dropped (only 1 sample).
        assert!(!stats.contains_key("family:support"));

        // __all__ has all 7.
        let all = stats.get(ALL_BUCKET).expect("__all__ bucket");
        assert_eq!(all["speed.scm"].n, 7);
        assert_eq!(all["speed.scm"].min, 150.0);
    }

    #[test]
    fn buckets_by_cohort_keys() {
        let entries: Vec<serde_json::Value> = (0..5)
            .map(|i| {
                json!({
                    "role": "Interceptor",
                    "manufacturer": "Aegis Dynamics",
                    "cargo_capacity": 0,
                    "speed": { "scm": 200 + i as i64 }
                })
            })
            .collect();
        let stats = build_category_stats(ReferenceCategory::Vehicle, entries);
        assert!(stats.contains_key("family:combat"));
        assert!(stats.contains_key("type:interceptor"));
        assert!(stats.contains_key("make:aegis-dynamics"));
        assert!(stats.contains_key("range:cargo:0-10"));
        assert!(stats.contains_key(ALL_BUCKET));
        assert_eq!(stats["type:interceptor"]["speed.scm"].n, 5);
    }

    #[test]
    fn quantiles_schema_matches_core_quantiles() {
        // The OpenAPI mirror `QuantilesSchema` must stay field-for-field
        // identical to `starstats_core::stats::Quantiles` — the wire format
        // serializes the core type, the spec describes the mirror. This
        // round-trips a real core `Quantiles` THROUGH the mirror's
        // deserializer and pins the exact key set, so adding or removing a
        // field on either type breaks this test.
        let q = Quantiles::from_values(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0])
            .expect("non-empty sample yields quantiles");
        let value = serde_json::to_value(q).expect("serialize core Quantiles");

        // Mirror must accept the core type's JSON verbatim.
        serde_json::from_value::<QuantilesSchema>(value.clone())
            .expect("QuantilesSchema must deserialize core Quantiles JSON");

        // Exact key set — drift on either side (added/removed field) trips here.
        let obj = value
            .as_object()
            .expect("Quantiles serializes to an object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected = ["min", "p10", "p25", "p50", "p75", "p90", "max", "n"];
        expected.sort_unstable();
        assert_eq!(
            keys, expected,
            "QuantilesSchema/Quantiles field set drifted"
        );
    }
}
