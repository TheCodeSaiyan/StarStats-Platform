//! Generic, pluggable enrichment seam.
//!
//! Companion framework to `reference_data.rs` (the primary catalogue
//! sync) and `location_enrichment.rs` (the first, bespoke enrichment
//! source). Where `location_enrichment` joins `starcitizen.tools`
//! taxonomy onto `reference_registry` location rows via a hand-written
//! `apply_location_taxonomies` store method + a hand-written cron, this
//! module generalises that pattern into a reusable trait + a generic
//! cron runner so new sources (Ship Matrix is the first) drop in with
//! a single `impl` + one spawn line.
//!
//! ## The seam
//!
//! An [`EnrichmentSource`] knows:
//!  - which [`ReferenceCategory`] it enriches ([`EnrichmentSource::category`]),
//!  - the metadata namespace key it writes under
//!    ([`EnrichmentSource::namespace`] — validated `^[a-z_]+$` by the
//!    store before it is interpolated into a JSONB path), and
//!  - how to fetch upstream + match the result against the existing
//!    rows ([`EnrichmentSource::fetch_and_match`]).
//!
//! The source owns its matching strategy entirely (fuzzy name, exact
//! `class_name`, slug) — the framework only cares about the resulting
//! `(class_name, blob)` pairs.
//!
//! ## Failure semantics
//!
//! Like `location_enrichment`, failure collapses to
//! [`EnrichmentOutcome::UpstreamUnavailable`]. The generic runner logs
//! and retains whatever enrichment is already in the store — stale
//! enrichment beats no enrichment, and an empty/garbage upstream must
//! never wipe a populated namespace (the store refuses empty batches
//! with [`crate::reference_store::ReferenceStoreError::EmptyBatch`]).
//!
//! ## Scope guard
//!
//! The existing `location_enrichment` cron is deliberately left on its
//! own bespoke path in v1 (it has a catalog-cache-refresh side-effect
//! the generic runner doesn't model). Collapsing it onto this seam is
//! a noted follow-up.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::reference_data::{ReferenceCategory, ReferenceEntry};
use crate::reference_store::ReferenceStore;

/// Outcome of an enrichment fetch-and-match pass.
///
/// `Entries` carries `(class_name, blob)` pairs the store will write
/// under the source's namespace. `serde_json::Value` doesn't impl
/// `Eq`, so this enum is `PartialEq` only — mirroring
/// [`crate::reference_data::ReferenceFetchOutcomeCategory`].
#[derive(Debug, Clone, PartialEq)]
pub enum EnrichmentOutcome {
    /// `(class_name, blob)` pairs to merge under the namespace.
    Entries(Vec<(String, serde_json::Value)>),
    /// Upstream unreachable / misbehaving — retain cached enrichment.
    UpstreamUnavailable,
}

/// A pluggable enrichment source. Implementers fetch from an external
/// system, match the result against the existing `reference_registry`
/// rows for their category, and return `(class_name, blob)` pairs.
#[async_trait]
pub trait EnrichmentSource: Send + Sync + 'static {
    /// Which registry category this source enriches.
    fn category(&self) -> ReferenceCategory;

    /// Metadata namespace key written under. MUST match `^[a-z_]+$`
    /// (the store re-validates before interpolating it into the JSONB
    /// path); e.g. `"ship_matrix"`.
    fn namespace(&self) -> &'static str;

    /// Human-readable label for log lines.
    fn name(&self) -> &'static str;

    /// Fetch upstream + match against `existing` rows → outcome.
    /// The source owns its matching strategy (fuzzy name, exact
    /// class_name, slug). `existing` is the full current row set for
    /// [`Self::category`] so the source can build whatever lookup
    /// index its matcher needs.
    async fn fetch_and_match(&self, existing: &[ReferenceEntry]) -> EnrichmentOutcome;
}

/// Sleep cadence after a successful enrichment pass.
const ENRICHMENT_OK: Duration = Duration::from_secs(24 * 3600);
/// Sleep cadence after a failed pass (transient upstream issue).
const ENRICHMENT_FAIL: Duration = Duration::from_secs(3600);

/// Generic best-effort cron runner for one [`EnrichmentSource`].
///
/// Loop shape mirrors the bespoke `location_enrichment` cron:
///  1. sleep `startup_offset` once (so it doesn't fight the primary
///     reference-data refresh at boot),
///  2. list the existing rows for the source's category,
///  3. `fetch_and_match`,
///  4. on `Entries`: `apply_enrichment` (which refuses an empty batch),
///  5. sleep 24h on success / 1h on any failure, then repeat.
///
/// Best-effort throughout: every failure logs and retains the cached
/// enrichment. Never panics out of the spawned task.
pub async fn run_enrichment_source(
    store: Arc<dyn ReferenceStore>,
    source: Arc<dyn EnrichmentSource>,
    startup_offset: Duration,
) {
    tokio::time::sleep(startup_offset).await;
    let category = source.category();
    let namespace = source.namespace();

    loop {
        let next = run_enrichment_pass(store.as_ref(), source.as_ref(), category, namespace).await;
        tokio::time::sleep(next).await;
    }
}

/// Single enrichment pass. Extracted so the cadence decision is a pure
/// function of one fetch-and-apply round, testable without the
/// surrounding sleep loop. Returns the sleep `Duration` to wait before
/// the next pass.
async fn run_enrichment_pass(
    store: &dyn ReferenceStore,
    source: &dyn EnrichmentSource,
    category: ReferenceCategory,
    namespace: &str,
) -> Duration {
    let existing = match store.list_category(category).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(
                error = %e,
                source = source.name(),
                category = category.as_str(),
                "enrichment: failed to list existing rows; retrying soon"
            );
            return ENRICHMENT_FAIL;
        }
    };

    match source.fetch_and_match(&existing).await {
        EnrichmentOutcome::Entries(pairs) => {
            // An empty match set is the same signal as an upstream
            // outage: do NOT call `apply_enrichment` (it would refuse
            // with `EmptyBatch` anyway), retain cached enrichment, and
            // retry soon. This keeps a transient zero-match parse from
            // looking like a hard error in logs.
            if pairs.is_empty() {
                tracing::warn!(
                    source = source.name(),
                    namespace,
                    "enrichment: zero matches this pass; retaining cached enrichment"
                );
                return ENRICHMENT_FAIL;
            }
            match store.apply_enrichment(category, namespace, &pairs).await {
                Ok(updated) => {
                    tracing::info!(
                        source = source.name(),
                        namespace,
                        matched = pairs.len(),
                        rows_updated = updated,
                        skipped_unmatched = pairs.len().saturating_sub(updated),
                        "enrichment applied"
                    );
                    ENRICHMENT_OK
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        source = source.name(),
                        namespace,
                        "enrichment apply failed; retaining cached enrichment"
                    );
                    ENRICHMENT_FAIL
                }
            }
        }
        EnrichmentOutcome::UpstreamUnavailable => {
            tracing::warn!(
                source = source.name(),
                namespace,
                "enrichment upstream unavailable; retaining cached enrichment"
            );
            ENRICHMENT_FAIL
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_store::test_support::MemoryReferenceStore;

    fn vehicle_row(class_name: &str, display_name: &str, slug: &str) -> ReferenceEntry {
        ReferenceEntry {
            category: ReferenceCategory::Vehicle,
            class_name: class_name.to_string(),
            display_name: display_name.to_string(),
            slug: Some(slug.to_string()),
            metadata: serde_json::json!({ "manufacturer": "Aegis Dynamics" }),
        }
    }

    /// Stub source returning a fixed outcome — lets us drive the
    /// generic runner's pass logic without a network call.
    struct StubSource {
        outcome: EnrichmentOutcome,
    }

    #[async_trait]
    impl EnrichmentSource for StubSource {
        fn category(&self) -> ReferenceCategory {
            ReferenceCategory::Vehicle
        }
        fn namespace(&self) -> &'static str {
            "ship_matrix"
        }
        fn name(&self) -> &'static str {
            "stub"
        }
        async fn fetch_and_match(&self, _existing: &[ReferenceEntry]) -> EnrichmentOutcome {
            self.outcome.clone()
        }
    }

    #[tokio::test]
    async fn pass_applies_entries_and_schedules_ok() {
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[vehicle_row(
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
                "aegis-avenger-stalker",
            )])
            .await
            .unwrap();
        let source = StubSource {
            outcome: EnrichmentOutcome::Entries(vec![(
                "AEGS_Avenger_Stalker".to_string(),
                serde_json::json!({ "specs": { "max_crew": 1 } }),
            )]),
        };

        let next =
            run_enrichment_pass(&store, &source, ReferenceCategory::Vehicle, "ship_matrix").await;
        assert_eq!(next, ENRICHMENT_OK);

        // The blob landed under metadata.ship_matrix.
        let entry = store
            .get_entry(ReferenceCategory::Vehicle, "AEGS_Avenger_Stalker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            entry.metadata["ship_matrix"]["specs"]["max_crew"]
                .as_i64()
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn pass_schedules_fail_on_upstream_unavailable() {
        let store = MemoryReferenceStore::new();
        let source = StubSource {
            outcome: EnrichmentOutcome::UpstreamUnavailable,
        };
        let next =
            run_enrichment_pass(&store, &source, ReferenceCategory::Vehicle, "ship_matrix").await;
        assert_eq!(next, ENRICHMENT_FAIL);
    }

    #[tokio::test]
    async fn pass_schedules_fail_on_empty_matches_without_wiping() {
        let store = MemoryReferenceStore::new();
        // Seed an already-enriched row.
        let mut row = vehicle_row("AEGS_Avenger_Stalker", "Aegis Avenger Stalker", "aas");
        row.metadata = serde_json::json!({
            "manufacturer": "Aegis Dynamics",
            "ship_matrix": { "specs": { "max_crew": 1 } }
        });
        store.upsert_entries(&[row]).await.unwrap();

        let source = StubSource {
            outcome: EnrichmentOutcome::Entries(vec![]),
        };
        let next =
            run_enrichment_pass(&store, &source, ReferenceCategory::Vehicle, "ship_matrix").await;
        assert_eq!(next, ENRICHMENT_FAIL);

        // The existing enrichment was NOT wiped by the empty pass.
        let entry = store
            .get_entry(ReferenceCategory::Vehicle, "AEGS_Avenger_Stalker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            entry.metadata["ship_matrix"]["specs"]["max_crew"]
                .as_i64()
                .unwrap(),
            1
        );
    }
}
