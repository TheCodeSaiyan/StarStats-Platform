//! Changelog auto-draft + publish flow (Phase 7, spec §8).
//!
//! On a `Shipped` transition (driven from `events.rs::ingest_event`),
//! `draft_for_shipped_transition` inserts a draft row capturing the
//! `previous_shipped_sha` -> `shipped_sha` pair. The body is a
//! placeholder until Phase 9 wires the real PR-title diffing through
//! the GitHub GraphQL reader.
//!
//! Admins publish drafts via `publish_with_notifications`. Publishing
//! flips the row's `published_at` stamp and best-effort fans out
//! subscriber notifications (today: tracing::info!) + an optional
//! Discord webhook ping (env-gated on `ROADMAP_DISCORD_WEBHOOK_URL`,
//! mirroring the Phase 6 writeback dry-run pattern: the HTTP POST is
//! deferred; we log the would-be payload).
//!
//! `spawn_purge_worker` runs a periodic sweep removing drafts older
//! than `ttl_days` (spec §8.4 default 30).

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::models::{ChannelName, RoadmapChangelogEntry};
use super::store::{DraftChangelog, RoadmapStore, RoadmapStoreError};

/// Errors surfaced to admin route handlers from the publish flow.
#[derive(Debug, thiserror::Error)]
pub enum ChangelogError {
    #[error("store error: {0}")]
    Store(#[from] RoadmapStoreError),
    #[error("changelog entry not found or already published")]
    NotFound,
}

/// Build a draft entry for a fresh `Shipped` transition. Today the
/// body is a placeholder; Phase 9 will replace it with real PR-title
/// diffing between `previous_shipped_sha` and `new_shipped_sha`.
///
/// `item_title` is taken from the `RoadmapItem` row so the draft is
/// self-descriptive even before an admin edits it.
pub async fn draft_for_shipped_transition(
    store: &dyn RoadmapStore,
    item_id: Uuid,
    channel: ChannelName,
    previous_shipped_sha: Option<&str>,
    new_shipped_sha: &str,
    item_title: &str,
) -> Result<RoadmapChangelogEntry, RoadmapStoreError> {
    let title = format!("{item_title} → {}", channel.as_str());
    let prev_label = previous_shipped_sha
        .map(short_sha)
        .unwrap_or_else(|| "(initial release)".to_string());
    let body = format!(
        "Shipped from {prev_label} to {new_sha}. Direct PR-title diffing is deferred to Phase 9.",
        new_sha = short_sha(new_shipped_sha),
    );
    store
        .draft_changelog(DraftChangelog {
            roadmap_item_id: item_id,
            channel,
            title: &title,
            body: &body,
            previous_shipped_sha,
            shipped_sha: Some(new_shipped_sha),
        })
        .await
}

/// Publish a draft and best-effort fan out notifications. The
/// notifications path is intentionally fail-silent: a publish that
/// succeeds in the DB never bubbles a webhook hiccup back to the
/// caller. The DB transition is the only load-bearing side effect.
pub async fn publish_with_notifications(
    store: &dyn RoadmapStore,
    entry_id: Uuid,
    published_by: &str,
) -> Result<RoadmapChangelogEntry, ChangelogError> {
    let entry = match store.publish_changelog(entry_id, published_by).await {
        Ok(e) => e,
        Err(RoadmapStoreError::NotFound) => return Err(ChangelogError::NotFound),
        Err(e) => return Err(ChangelogError::Store(e)),
    };

    // Fan out subscriber notifications (spec §6.2). Today: tracing-
    // only — the cross-device notification framework is the parent
    // dependency. Counting subscribers anchors the metric so an
    // outage in the subscriber store is visible in logs.
    match store.list_subscribers_for_item(entry.roadmap_item_id).await {
        Ok(subs) => {
            tracing::info!(
                entry_id = %entry.id,
                item_id = %entry.roadmap_item_id,
                subscriber_count = subs.len(),
                "roadmap changelog published; subscriber fan-out queued"
            );
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                entry_id = %entry.id,
                "roadmap changelog published; subscriber list lookup failed"
            );
        }
    }

    // Discord webhook (spec §8.4). Missing env = fail silent; the
    // env-gated HTTP POST is deferred to a follow-up (mirrors Phase 6
    // writeback's dry-run pattern). Reading the var on every publish
    // means an admin can flip the channel without restarting the
    // server.
    if let Ok(url) = std::env::var("ROADMAP_DISCORD_WEBHOOK_URL") {
        if !url.trim().is_empty() {
            tracing::info!(
                entry_id = %entry.id,
                webhook_url = %redact_webhook(&url),
                title = %entry.title,
                "roadmap changelog: would POST Discord webhook (deferred)"
            );
        }
    }

    Ok(entry)
}

/// Spawn a periodic purge worker. Runs every `interval` and removes
/// draft rows older than `ttl_days`. Returns the JoinHandle so the
/// caller can abort on shutdown.
pub fn spawn_purge_worker(
    store: Arc<dyn RoadmapStore>,
    ttl_days: i64,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let cutoff = Utc::now() - chrono::Duration::days(ttl_days);
            match store.purge_old_drafts(cutoff).await {
                Ok(n) if n > 0 => {
                    tracing::info!(purged = n, ttl_days, "roadmap changelog: draft purge sweep");
                }
                Ok(_) => {
                    tracing::debug!(ttl_days, "roadmap changelog: draft purge sweep (no rows)");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "roadmap changelog: draft purge failed");
                }
            }
        }
    })
}

/// Trim a SHA to 7 chars for human-friendly inline rendering. Avoids
/// truncation panics on already-short inputs.
fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// Strip the webhook secret from a Discord URL before logging. Discord
/// webhook URLs are sensitive bearer-equivalents; we never log the
/// full URL.
fn redact_webhook(url: &str) -> String {
    if let Some(idx) = url.rfind('/') {
        if idx + 1 < url.len() {
            return format!("{}/<redacted>", &url[..idx]);
        }
    }
    "<redacted>".to_string()
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::store::test_support::MemoryRoadmapStore;
    use super::super::store::UpsertRoadmapItem;
    use super::*;
    use chrono::Duration as ChronoDuration;

    async fn seed_item(store: &MemoryRoadmapStore, slug: &str, title: &str) -> Uuid {
        let surfaces: Vec<String> = vec![];
        store
            .upsert_item(UpsertRoadmapItem {
                github_project_item_id: &format!("PVTI_{slug}"),
                slug,
                title,
                summary: None,
                category: None,
                eta_band: None,
                surfaces: &surfaces,
                parent_id: None,
                links: None,
                public: true,
            })
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn draft_for_shipped_transition_creates_unpublished_entry() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store, "feature-x", "Feature X").await;
        let entry = draft_for_shipped_transition(
            &store,
            item_id,
            ChannelName::Live,
            Some("0123456789abcdef"),
            "fedcba9876543210",
            "Feature X",
        )
        .await
        .unwrap();
        assert_eq!(entry.roadmap_item_id, item_id);
        assert_eq!(entry.channel, ChannelName::Live);
        assert!(entry.published_at.is_none(), "draft is unpublished");
        assert!(entry.title.contains("Feature X"));
        assert!(entry.title.contains("live"));
        // Body mentions both short SHAs.
        assert!(entry.body.contains("0123456"));
        assert!(entry.body.contains("fedcba9"));
    }

    #[tokio::test]
    async fn draft_with_no_previous_sha_renders_initial_release() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store, "fresh", "Fresh feature").await;
        let entry = draft_for_shipped_transition(
            &store,
            item_id,
            ChannelName::Beta,
            None,
            "deadbeef",
            "Fresh feature",
        )
        .await
        .unwrap();
        assert!(entry.body.contains("initial release"));
    }

    #[tokio::test]
    async fn publish_flips_published_at_and_records_publisher() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store, "pub-test", "Publish test").await;
        let draft = draft_for_shipped_transition(
            &store,
            item_id,
            ChannelName::Live,
            None,
            "abc1234",
            "Publish test",
        )
        .await
        .unwrap();
        let published = publish_with_notifications(&store, draft.id, "admin-handle")
            .await
            .unwrap();
        assert!(published.published_at.is_some());
        assert_eq!(published.published_by.as_deref(), Some("admin-handle"));
    }

    #[tokio::test]
    async fn publish_already_published_returns_not_found() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store, "double-pub", "Double publish").await;
        let draft = draft_for_shipped_transition(
            &store,
            item_id,
            ChannelName::Live,
            None,
            "abc1234",
            "Double publish",
        )
        .await
        .unwrap();
        publish_with_notifications(&store, draft.id, "admin-1")
            .await
            .unwrap();
        let err = publish_with_notifications(&store, draft.id, "admin-2")
            .await
            .unwrap_err();
        assert!(matches!(err, ChangelogError::NotFound));
    }

    #[tokio::test]
    async fn purge_removes_old_drafts_only() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store, "purge-test", "Purge test").await;

        // Fresh draft -- should survive.
        let fresh = draft_for_shipped_transition(
            &store,
            item_id,
            ChannelName::Live,
            None,
            "newsha",
            "Purge test",
        )
        .await
        .unwrap();

        // Published draft -- should survive even if old.
        let published_draft = draft_for_shipped_transition(
            &store,
            item_id,
            ChannelName::Beta,
            None,
            "pubsha",
            "Purge test",
        )
        .await
        .unwrap();
        publish_with_notifications(&store, published_draft.id, "admin")
            .await
            .unwrap();

        // Cutoff in the FUTURE: every draft is "older than future" so
        // every unpublished draft is purged; the published row stays.
        let future = Utc::now() + ChronoDuration::days(1);
        let n = store.purge_old_drafts(future).await.unwrap();
        assert_eq!(n, 1, "exactly one draft (the unpublished one) was purged");
        // The published row is still findable.
        let still = store.get_changelog_entry(published_draft.id).await.unwrap();
        assert!(still.is_some());
        // The fresh draft is gone.
        let gone = store.get_changelog_entry(fresh.id).await.unwrap();
        assert!(gone.is_none());
    }

    #[tokio::test]
    async fn edit_draft_updates_title_and_body() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store, "edit-test", "Edit test").await;
        let draft = draft_for_shipped_transition(
            &store,
            item_id,
            ChannelName::Live,
            None,
            "abc",
            "Edit test",
        )
        .await
        .unwrap();
        let edited = store
            .edit_changelog_draft(draft.id, "New title", "New body")
            .await
            .unwrap();
        assert_eq!(edited.title, "New title");
        assert_eq!(edited.body, "New body");
        // Edits to an already-published entry return NotFound.
        publish_with_notifications(&store, draft.id, "admin")
            .await
            .unwrap();
        let err = store
            .edit_changelog_draft(draft.id, "after publish", "no")
            .await;
        assert!(matches!(err, Err(RoadmapStoreError::NotFound)));
    }

    #[test]
    fn redact_webhook_strips_secret_segment() {
        let red = redact_webhook("https://discord.com/api/webhooks/12345/SECRET-TOKEN-DATA-HERE");
        assert!(red.ends_with("/<redacted>"));
        assert!(!red.contains("SECRET-TOKEN-DATA-HERE"));
    }
}
