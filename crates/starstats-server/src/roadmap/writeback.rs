//! Writeback worker for the roadmap pipeline (Phase 6 / spec §6.3).
//!
//! For each live roadmap item, compute the current `votes` and
//! `subscriber_count` from our store, then push the values back to
//! the GitHub Project so the curator sees them on the board itself.
//!
//! Phase 6 simplification: this module LOGS what would be written
//! but does not yet call the GitHub mutation. The actual
//! `updateProjectV2ItemFieldValue` GraphQL mutation lives behind a
//! Phase 9 task — wiring the mutation needs (a) the project field
//! IDs cached at startup, (b) a write-side path through the
//! `GitHubGraphQLClient`, and (c) backoff + audit-log on final
//! failure per spec §6.3. None of that is on the Phase 6 critical
//! path; the read-side aggregation pinned here is.
//!
//! `writeback_once` is the unit; `spawn_writeback` wraps it in a
//! 5-min loop matching `sync::spawn_reconciler`.

use std::sync::Arc;
use std::time::Duration;

use super::github_graphql::GitHubReader;
use super::store::RoadmapStore;

/// One pass over every live item. Coalesces per-item writes so a
/// vote burst doesn't translate into per-vote GitHub traffic (spec
/// §6.3 "one write per item per batch").
///
/// Returns the number of items that would have been written. Tests
/// assert on this rather than on the (currently absent) mutation
/// payload.
pub async fn writeback_once(
    store: &dyn RoadmapStore,
    _reader: &dyn GitHubReader,
    project_id: &str,
) -> WritebackStats {
    let items = match store.list_items(false).await {
        Ok(items) => items,
        Err(e) => {
            tracing::warn!(error = %e, project_id, "writeback list_items failed");
            return WritebackStats::default();
        }
    };
    let mut stats = WritebackStats {
        items_seen: items.len(),
        ..Default::default()
    };

    for item in items {
        let tally = match store.count_votes(item.id).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    item_id = %item.id,
                    slug = %item.slug,
                    "writeback count_votes failed; skipping item"
                );
                stats.items_skipped += 1;
                continue;
            }
        };
        let subscribers = match store.list_subscribers_for_item(item.id).await {
            Ok(subs) => subs.len(),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    item_id = %item.id,
                    slug = %item.slug,
                    "writeback list_subscribers failed; skipping item"
                );
                stats.items_skipped += 1;
                continue;
            }
        };

        // TODO Phase 9: actually POST the updateProjectV2ItemFieldValue
        // mutation here. For now we log the intended payload so the
        // operator sees the aggregation working pre-cutover.
        tracing::info!(
            project_id,
            item_id = %item.id,
            slug = %item.slug,
            github_project_item_id = %item.github_project_item_id,
            votes = tally.votes,
            subscribers,
            "roadmap writeback (phase-6 dry run)"
        );
        stats.items_written += 1;
    }

    stats
}

/// Counters returned from `writeback_once` for observability + tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WritebackStats {
    pub items_seen: usize,
    /// Items that passed aggregation and are eligible to be pushed.
    /// Until Phase 9 wires the mutation, NOTHING is actually written --
    /// this is a count of intent, not of effect, and the summary log
    /// says so.
    pub items_written: usize,
    pub items_skipped: usize,
}

/// Spawn the 5-minute writeback loop. Matches `sync::spawn_reconciler`
/// in shape so the operator interleaves the two cadences mentally as a
/// pair: the reconciler pulls; the writeback pushes.
pub fn spawn_writeback(
    store: Arc<dyn RoadmapStore>,
    reader: Arc<dyn GitHubReader>,
    project_id: String,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let stats = writeback_once(&*store, &*reader, &project_id).await;
            tracing::info!(
                items_seen = stats.items_seen,
                items_written = stats.items_written,
                items_skipped = stats.items_skipped,
                // Until Phase 9 lands the mutation, `items_written`
                // counts items that WOULD be pushed -- nothing reaches
                // GitHub. Without this marker the summary line reads
                // "17 items written" and an operator reasonably
                // concludes the sync is live. Flip to false in Phase 9.
                dry_run = true,
                "roadmap writeback ok (dry run: nothing pushed to GitHub)"
            );
        }
    })
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::github_graphql::{GitHubError, ProjectItem};
    use super::super::store::test_support::MemoryRoadmapStore;
    use super::super::store::UpsertRoadmapItem;
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    /// Counter-backed reader that records every call so the test can
    /// assert the writeback didn't accidentally hit the GitHub seam
    /// (Phase 6 is read-only on the reader side -- writes go through
    /// the absent mutation method).
    struct CountingReader {
        list_calls: AtomicUsize,
        get_calls: AtomicUsize,
    }

    impl CountingReader {
        fn new() -> Self {
            Self {
                list_calls: AtomicUsize::new(0),
                get_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl GitHubReader for CountingReader {
        async fn list_project_items(
            &self,
            _project_id: &str,
        ) -> Result<Vec<ProjectItem>, GitHubError> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }

        async fn get_project_item(&self, _item_id: &str) -> Result<ProjectItem, GitHubError> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            Err(GitHubError::Schema("no item".into()))
        }

        async fn list_project_item_ids_for_issue(
            &self,
            _issue_id: &str,
            _project_id: &str,
        ) -> Result<Vec<String>, GitHubError> {
            Ok(Vec::new())
        }
    }

    async fn seed(store: &MemoryRoadmapStore, slug: &str) -> Uuid {
        let surfaces: Vec<String> = vec![];
        let item = store
            .upsert_item(UpsertRoadmapItem {
                github_project_item_id: &format!("PVTI_{slug}"),
                slug,
                title: slug,
                summary: None,
                category: None,
                eta_band: None,
                surfaces: &surfaces,
                parent_id: None,
                links: None,
                public: true,
            })
            .await
            .unwrap();
        item.id
    }

    #[tokio::test]
    async fn writeback_once_iterates_all_items() {
        let store = MemoryRoadmapStore::new();
        let _a = seed(&store, "alpha").await;
        let _b = seed(&store, "beta").await;
        let _c = seed(&store, "gamma").await;
        let reader = CountingReader::new();

        let stats = writeback_once(&store, &reader, "PROJ_TEST").await;
        assert_eq!(stats.items_seen, 3);
        assert_eq!(stats.items_written, 3);
        assert_eq!(stats.items_skipped, 0);
        // Phase 6 reads from the store only -- the GitHub seam is
        // untouched. Phase 9 will flip this when the mutation lands.
        assert_eq!(reader.list_calls.load(Ordering::SeqCst), 0);
        assert_eq!(reader.get_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn writeback_once_aggregates_votes_and_subscribers() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed(&store, "hot-feature").await;

        // Cast 3 distinct votes + 2 subscribers.
        for _ in 0..3 {
            store.cast_vote(Uuid::now_v7(), item_id).await.unwrap();
        }
        for _ in 0..2 {
            store.subscribe(Uuid::now_v7(), item_id).await.unwrap();
        }

        let reader = CountingReader::new();
        let stats = writeback_once(&store, &reader, "PROJ_AGG").await;
        // Single item -> single write attempt.
        assert_eq!(stats.items_seen, 1);
        assert_eq!(stats.items_written, 1);

        // And the store actually holds what the writeback logged.
        assert_eq!(store.count_votes(item_id).await.unwrap().votes, 3);
        assert_eq!(
            store
                .list_subscribers_for_item(item_id)
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn writeback_once_with_no_items_is_a_noop() {
        let store = MemoryRoadmapStore::new();
        let reader = CountingReader::new();
        let stats = writeback_once(&store, &reader, "PROJ_EMPTY").await;
        assert_eq!(stats.items_seen, 0);
        assert_eq!(stats.items_written, 0);
        assert_eq!(stats.items_skipped, 0);
    }
}
