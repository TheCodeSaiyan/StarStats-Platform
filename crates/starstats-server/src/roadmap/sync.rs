//! Roadmap sync engine: reconciler + GitHub Projects v2 webhook.
//!
//! Phase 3 of the roadmap pipeline (spec §3.4 / §3.5). Pulls Project
//! items from GitHub on a 5-minute reconciliation tick and on each
//! inbound `projects_v2_item` webhook event, maps them via
//! [`super::mapper`], and applies the resulting upserts to the local
//! `RoadmapStore`.
//!
//! Channel statuses owned by the CI pipeline (Phase 4) are NEVER
//! overwritten here — the sync only fills in channels newly discovered
//! from labels (with `status = Proposed, build_health = Unknown`) and
//! archives channels that have disappeared from the labels (spec §2.6).
//!
//! The webhook signature scheme is GitHub's standard
//! `X-Hub-Signature-256: sha256=<hex>` HMAC over the raw request body.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use super::github_graphql::{GitHubError, GitHubReader};
use super::mapper::{self, MappedItem};
use super::models::{BuildHealth, ChannelName, ChannelStatus, RoadmapStatus};
use super::store::{RoadmapStore, RoadmapStoreError, UpsertChannelStatus, UpsertRoadmapItem};

type HmacSha256 = Hmac<Sha256>;

// ---------- errors ----------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("github error: {0}")]
    GitHub(#[from] GitHubError),
    #[error("store error: {0}")]
    Store(#[from] RoadmapStoreError),
    #[error("webhook signature invalid")]
    SignatureInvalid,
    #[error("webhook signature malformed: {0}")]
    SignatureMalformed(String),
    #[error("webhook payload parse: {0}")]
    PayloadParse(String),
    #[error("webhook for unknown action: {0}")]
    UnknownAction(String),
    #[error("webhook for unknown event: {0}")]
    UnknownEvent(String),
}

// ---------- reconcile stats -------------------------------------------------

/// Counters returned by [`reconcile_once`]. Useful for tracing /
/// `/metrics` exposure later; not currently emitted as metric values.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileStats {
    pub items_seen: usize,
    pub items_upserted: usize,
    pub channels_added: usize,
    pub channels_archived: usize,
}

// ---------- one-shot reconcile ---------------------------------------------

/// Run a full reconciliation pass: list every Project item, map it,
/// upsert into the store, and reconcile channel discovery.
pub async fn reconcile_once(
    store: &dyn RoadmapStore,
    reader: &dyn GitHubReader,
    project_id: &str,
) -> Result<ReconcileStats, SyncError> {
    let items = reader.list_project_items(project_id).await?;
    let mut stats = ReconcileStats {
        items_seen: items.len(),
        ..ReconcileStats::default()
    };
    for pi in items {
        let mapped = mapper::map_project_item(&pi);
        let one = apply_one(store, &mapped).await?;
        stats.items_upserted += 1;
        stats.channels_added += one.channels_added;
        stats.channels_archived += one.channels_archived;
    }
    Ok(stats)
}

#[derive(Debug, Default)]
struct OneStats {
    channels_added: usize,
    channels_archived: usize,
}

async fn apply_one(store: &dyn RoadmapStore, mapped: &MappedItem) -> Result<OneStats, SyncError> {
    // Items with no title (e.g. unknown content typename) — skip.
    if mapped.title.is_empty() {
        return Ok(OneStats::default());
    }

    let surfaces_owned: Vec<String> = mapped.surfaces.clone();
    let item = store
        .upsert_item(UpsertRoadmapItem {
            github_project_item_id: &mapped.github_project_item_id,
            slug: &mapped.slug,
            title: &mapped.title,
            summary: mapped.summary.as_deref(),
            category: mapped.category.as_deref(),
            eta_band: mapped.eta_band.map(|e| e.as_str()),
            surfaces: &surfaces_owned,
            parent_id: None, // Phase 3 doesn't yet wire hierarchy (spec §1.6).
            links: None,
            public: mapped.public,
        })
        .await?;

    let existing = store.list_channel_statuses(item.id).await?;
    let stats = reconcile_channels(store, item.id, &existing, &mapped.channels).await?;
    Ok(stats)
}

/// Diff `existing` channel statuses against `desired` channel set,
/// inserting Proposed/Unknown rows for additions and archiving
/// disappeared channels.
///
/// Never touches the `status` or `build_health` of an existing row —
/// the CI event pipeline (Phase 4) owns those.
async fn reconcile_channels(
    store: &dyn RoadmapStore,
    roadmap_item_id: Uuid,
    existing: &[ChannelStatus],
    desired: &[ChannelName],
) -> Result<OneStats, SyncError> {
    let mut stats = OneStats::default();

    // Additions: channels in `desired` but not yet in `existing`.
    for &channel in desired {
        if !existing.iter().any(|s| s.channel == channel) {
            store
                .upsert_channel_status(UpsertChannelStatus {
                    roadmap_item_id,
                    channel,
                    status: RoadmapStatus::Proposed,
                    build_health: BuildHealth::Unknown,
                    build_id: None,
                    commit_sha: None,
                    deployed_at: None,
                    ci_run_url: None,
                    previous_shipped_sha: None,
                    last_event_id: None,
                })
                .await?;
            stats.channels_added += 1;
        }
    }

    // Archives: channels in `existing` but no longer in `desired`.
    for s in existing {
        if !desired.contains(&s.channel) {
            // archive_channel returns NotFound if there's no live row —
            // a race we shouldn't ever hit because we just listed
            // them, but stay defensive.
            match store.archive_channel(roadmap_item_id, s.channel).await {
                Ok(()) => stats.channels_archived += 1,
                Err(RoadmapStoreError::NotFound) => {
                    tracing::warn!(
                        roadmap_item_id = %roadmap_item_id,
                        channel = s.channel.as_str(),
                        "archive_channel raced; live row vanished between list and archive"
                    );
                }
                Err(other) => return Err(other.into()),
            }
        }
    }

    Ok(stats)
}

// ---------- webhook ---------------------------------------------------------

/// GitHub `projects_v2_item` webhook payload (slim — we only need a
/// couple of fields).
#[derive(Debug, Deserialize)]
pub struct WebhookPayload {
    pub action: String,
    pub projects_v2_item: WebhookItem,
}

#[derive(Debug, Deserialize)]
pub struct WebhookItem {
    pub node_id: String,
}

/// GitHub `issues` webhook payload (slim — node_id is the only thing
/// we use; the rest comes from a follow-up GraphQL read). The receiver
/// uses this to discover which Project items contain the issue and
/// re-syncs them; that handles `surface/*` and `channel/*` label
/// changes which would otherwise only propagate on the 5-min
/// reconciler tick.
#[derive(Debug, Deserialize)]
pub struct IssueWebhookPayload {
    pub action: String,
    pub issue: IssueRef,
}

#[derive(Debug, Deserialize)]
pub struct IssueRef {
    pub node_id: String,
}

/// Verify a GitHub-style `X-Hub-Signature-256: sha256=<hex>` header
/// against the raw body. Constant-time compare via `subtle`.
pub fn verify_github_webhook_signature(
    secret: &[u8],
    body: &[u8],
    signature_header: &str,
) -> Result<(), SyncError> {
    let expected_hex = signature_header
        .strip_prefix("sha256=")
        .ok_or_else(|| SyncError::SignatureMalformed("missing sha256= prefix".into()))?;
    let expected_bytes = hex::decode(expected_hex)
        .map_err(|e| SyncError::SignatureMalformed(format!("hex decode: {e}")))?;
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|e| SyncError::SignatureMalformed(format!("hmac init: {e}")))?;
    mac.update(body);
    let computed = mac.finalize().into_bytes();
    if computed.ct_eq(expected_bytes.as_slice()).into() {
        Ok(())
    } else {
        Err(SyncError::SignatureInvalid)
    }
}

/// Handle one verified webhook payload. The caller is responsible for
/// running [`verify_github_webhook_signature`] first — this function
/// trusts the body it's given.
///
/// `event` is the value of GitHub's `X-GitHub-Event` header. Only
/// `projects_v2_item` and `issues` are recognized; other events
/// resolve to [`SyncError::UnknownEvent`] (which the route layer
/// treats as 204 + log, matching the unknown-action behaviour).
///
/// `project_id` is the configured roadmap Project node id. Used by
/// the `issues` branch to scope the linked-items lookup; the
/// `projects_v2_item` branch ignores it (the payload already names
/// the item directly).
pub async fn handle_webhook_verified(
    store: &dyn RoadmapStore,
    reader: &dyn GitHubReader,
    event: &str,
    project_id: &str,
    body: &[u8],
) -> Result<(), SyncError> {
    match event {
        "projects_v2_item" => handle_projects_v2_item_event(store, reader, body).await,
        "issues" => handle_issue_event(store, reader, project_id, body).await,
        other => Err(SyncError::UnknownEvent(other.to_string())),
    }
}

async fn handle_projects_v2_item_event(
    store: &dyn RoadmapStore,
    reader: &dyn GitHubReader,
    body: &[u8],
) -> Result<(), SyncError> {
    let payload: WebhookPayload =
        serde_json::from_slice(body).map_err(|e| SyncError::PayloadParse(e.to_string()))?;
    match payload.action.as_str() {
        // Deletions and archive are both "remove from the live set" —
        // we soft-delete locally either way. Restoring an archived
        // item is treated like an edit (re-fetch + upsert).
        "deleted" | "archived" => {
            store
                .soft_delete_by_github_id(&payload.projects_v2_item.node_id)
                .await?;
            Ok(())
        }
        // Every other lifecycle event re-fetches the item and applies
        // through the same mapper as the reconciler — keeps the code
        // paths convergent.
        "created" | "edited" | "reordered" | "converted" | "restored" => {
            let pi = reader
                .get_project_item(&payload.projects_v2_item.node_id)
                .await?;
            let mapped = mapper::map_project_item(&pi);
            apply_one(store, &mapped).await?;
            Ok(())
        }
        other => Err(SyncError::UnknownAction(other.to_string())),
    }
}

/// Handle an `issues` event by finding every Project item that
/// references this Issue (filtered to our configured `project_id`)
/// and re-applying the mapper for each. Closes the latency gap on
/// `surface/*` and `channel/*` label changes, which would otherwise
/// only land on the 5-min reconciler tick — see the spec's
/// "Webhook + reconciliation" section.
async fn handle_issue_event(
    store: &dyn RoadmapStore,
    reader: &dyn GitHubReader,
    project_id: &str,
    body: &[u8],
) -> Result<(), SyncError> {
    let payload: IssueWebhookPayload =
        serde_json::from_slice(body).map_err(|e| SyncError::PayloadParse(e.to_string()))?;
    // Only label-affecting + creation actions trigger a resync.
    // `assigned`, `commented`, `closed`, `pinned`, `milestoned`, etc.
    // don't change anything the mapper cares about; ignoring them
    // keeps unnecessary GraphQL round-trips off the receiver.
    match payload.action.as_str() {
        "labeled" | "unlabeled" | "opened" | "reopened" => {
            let item_ids = reader
                .list_project_item_ids_for_issue(&payload.issue.node_id, project_id)
                .await?;
            for item_id in item_ids {
                let pi = reader.get_project_item(&item_id).await?;
                let mapped = mapper::map_project_item(&pi);
                apply_one(store, &mapped).await?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

// ---------- reconciler spawn ----------------------------------------------

/// Spawn the 5-minute reconciler loop. Returns the join handle so the
/// caller can abort on shutdown.
pub fn spawn_reconciler(
    store: Arc<dyn RoadmapStore>,
    reader: Arc<dyn GitHubReader>,
    project_id: String,
    interval: StdDuration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match reconcile_once(&*store, &*reader, &project_id).await {
                Ok(stats) => {
                    tracing::info!(
                        items_seen = stats.items_seen,
                        items_upserted = stats.items_upserted,
                        channels_added = stats.channels_added,
                        channels_archived = stats.channels_archived,
                        "roadmap reconcile ok"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "roadmap reconcile failed");
                }
            }
        }
    })
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::github_graphql::{ProjectItem, ProjectItemContent};
    use super::super::store::test_support::MemoryRoadmapStore;
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FakeReader {
        items: Mutex<Vec<ProjectItem>>,
        single: Mutex<HashMap<String, ProjectItem>>,
        // issue_node_id → list of (project_item_id, project_id) pairs.
        // An issue can be on multiple Projects across the org; the
        // reader's filter narrows to the configured project_id.
        issue_links: Mutex<HashMap<String, Vec<(String, String)>>>,
    }

    impl FakeReader {
        fn new(items: Vec<ProjectItem>) -> Self {
            let by_id: HashMap<String, ProjectItem> =
                items.iter().map(|p| (p.id.clone(), p.clone())).collect();
            Self {
                items: Mutex::new(items),
                single: Mutex::new(by_id),
                issue_links: Mutex::new(HashMap::new()),
            }
        }

        fn put_single(&self, p: ProjectItem) {
            self.single.lock().unwrap().insert(p.id.clone(), p);
        }

        fn put_issue_link(&self, issue_id: &str, item_id: &str, project_id: &str) {
            self.issue_links
                .lock()
                .unwrap()
                .entry(issue_id.to_string())
                .or_default()
                .push((item_id.to_string(), project_id.to_string()));
        }
    }

    #[async_trait]
    impl GitHubReader for FakeReader {
        async fn list_project_items(
            &self,
            _project_id: &str,
        ) -> Result<Vec<ProjectItem>, GitHubError> {
            Ok(self.items.lock().unwrap().clone())
        }

        async fn get_project_item(&self, item_id: &str) -> Result<ProjectItem, GitHubError> {
            self.single
                .lock()
                .unwrap()
                .get(item_id)
                .cloned()
                .ok_or_else(|| GitHubError::Schema(format!("not found: {item_id}")))
        }

        async fn list_project_item_ids_for_issue(
            &self,
            issue_id: &str,
            project_id: &str,
        ) -> Result<Vec<String>, GitHubError> {
            let links = self.issue_links.lock().unwrap();
            Ok(links
                .get(issue_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, pid)| pid == project_id)
                .map(|(iid, _)| iid)
                .collect())
        }
    }

    fn issue(id: &str, title: &str, labels: Vec<String>) -> ProjectItem {
        ProjectItem {
            id: id.to_string(),
            content: ProjectItemContent::Issue {
                title: title.to_string(),
                body: format!("Body of {title}."),
                url: format!("https://example/{id}"),
                labels,
            },
            custom_fields: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn reconcile_creates_items_and_initial_channels() {
        let reader = FakeReader::new(vec![
            issue(
                "PVTI_a",
                "Item A",
                vec!["channel/live".into(), "channel/beta".into()],
            ),
            issue("PVTI_b", "Item B", vec!["channel/live".into()]),
        ]);
        let store = MemoryRoadmapStore::new();
        let stats = reconcile_once(&store, &reader, "PVT_proj").await.unwrap();
        assert_eq!(stats.items_seen, 2);
        assert_eq!(stats.items_upserted, 2);
        assert_eq!(stats.channels_added, 3);
        assert_eq!(stats.channels_archived, 0);

        let a = store.get_item_by_slug("item-a").await.unwrap().unwrap();
        let a_channels = store.list_channel_statuses(a.id).await.unwrap();
        assert_eq!(a_channels.len(), 2);
        for c in a_channels {
            assert_eq!(c.status, RoadmapStatus::Proposed);
            assert_eq!(c.build_health, BuildHealth::Unknown);
        }
    }

    #[tokio::test]
    async fn reconcile_preserves_existing_channel_status_set_by_ci() {
        // Simulate: first reconcile creates Item A's `live` channel as
        // Proposed; then CI pipeline (we'll just call upsert directly)
        // bumps it to Building; then a second reconcile must NOT
        // overwrite the Building state.
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![issue("PVTI_a", "Item A", vec!["channel/live".into()])]);
        reconcile_once(&store, &reader, "PVT_proj").await.unwrap();
        let a = store.get_item_by_slug("item-a").await.unwrap().unwrap();
        // CI-like override.
        store
            .upsert_channel_status(UpsertChannelStatus {
                roadmap_item_id: a.id,
                channel: ChannelName::Live,
                status: RoadmapStatus::Building,
                build_health: BuildHealth::Passing,
                build_id: Some("b1"),
                commit_sha: Some("deadbeef"),
                deployed_at: None,
                ci_run_url: None,
                previous_shipped_sha: None,
                last_event_id: Some("evt-1"),
            })
            .await
            .unwrap();
        // Second reconcile — same labels, no churn.
        reconcile_once(&store, &reader, "PVT_proj").await.unwrap();
        let chans = store.list_channel_statuses(a.id).await.unwrap();
        assert_eq!(chans.len(), 1);
        assert_eq!(chans[0].status, RoadmapStatus::Building);
        assert_eq!(chans[0].build_health, BuildHealth::Passing);
        assert_eq!(chans[0].commit_sha.as_deref(), Some("deadbeef"));
    }

    #[tokio::test]
    async fn reconcile_archives_channel_removed_from_labels() {
        let store = MemoryRoadmapStore::new();
        // First pass: item with live + beta.
        let reader1 = FakeReader::new(vec![issue(
            "PVTI_a",
            "Item A",
            vec!["channel/live".into(), "channel/beta".into()],
        )]);
        reconcile_once(&store, &reader1, "PVT_proj").await.unwrap();
        let a = store.get_item_by_slug("item-a").await.unwrap().unwrap();
        assert_eq!(store.list_channel_statuses(a.id).await.unwrap().len(), 2);

        // Second pass: beta removed.
        let reader2 = FakeReader::new(vec![issue("PVTI_a", "Item A", vec!["channel/live".into()])]);
        let stats = reconcile_once(&store, &reader2, "PVT_proj").await.unwrap();
        assert_eq!(stats.channels_archived, 1);
        let live = store.list_channel_statuses(a.id).await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].channel, ChannelName::Live);
    }

    #[tokio::test]
    async fn webhook_deleted_soft_deletes_local() {
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![issue("PVTI_a", "Item A", vec!["channel/live".into()])]);
        reconcile_once(&store, &reader, "PVT_proj").await.unwrap();
        assert!(store.get_item_by_slug("item-a").await.unwrap().is_some());
        let body = br#"{"action":"deleted","projects_v2_item":{"node_id":"PVTI_a"}}"#.to_vec();
        handle_webhook_verified(&store, &reader, "projects_v2_item", "PVT_proj", &body)
            .await
            .unwrap();
        // Soft-deleted: get_item_by_slug filters deleted_at.
        assert!(store.get_item_by_slug("item-a").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn webhook_edited_refetches_and_upserts() {
        let store = MemoryRoadmapStore::new();
        // Pre-seed: nothing.
        let reader = FakeReader::new(vec![]);
        reader.put_single(issue("PVTI_a", "Item A", vec!["channel/live".into()]));
        let body = br#"{"action":"edited","projects_v2_item":{"node_id":"PVTI_a"}}"#.to_vec();
        handle_webhook_verified(&store, &reader, "projects_v2_item", "PVT_proj", &body)
            .await
            .unwrap();
        let a = store
            .get_item_by_slug("item-a")
            .await
            .unwrap()
            .expect("upserted");
        let chans = store.list_channel_statuses(a.id).await.unwrap();
        assert_eq!(chans.len(), 1);
        assert_eq!(chans[0].channel, ChannelName::Live);
    }

    #[tokio::test]
    async fn webhook_unknown_action_errors() {
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![]);
        let body = br#"{"action":"reviewed","projects_v2_item":{"node_id":"PVTI_a"}}"#.to_vec();
        let err = handle_webhook_verified(&store, &reader, "projects_v2_item", "PVT_proj", &body)
            .await
            .unwrap_err();
        assert!(matches!(err, SyncError::UnknownAction(_)));
    }

    // ---------- signature ----------

    fn compute_sig(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let out = mac.finalize().into_bytes();
        format!("sha256={}", hex::encode(out))
    }

    #[test]
    fn webhook_signature_ok_round_trip() {
        let secret = b"shh-very-secret";
        let body = br#"{"action":"created"}"#;
        let sig = compute_sig(secret, body);
        verify_github_webhook_signature(secret, body, &sig).unwrap();
    }

    #[test]
    fn webhook_signature_rejects_tampered_body() {
        let secret = b"shh-very-secret";
        let body = br#"{"action":"created"}"#;
        let sig = compute_sig(secret, body);
        let tampered = br#"{"action":"deleted"}"#;
        let err = verify_github_webhook_signature(secret, tampered, &sig).unwrap_err();
        assert!(matches!(err, SyncError::SignatureInvalid));
    }

    #[test]
    fn webhook_signature_rejects_malformed_header() {
        let secret = b"shh";
        let err = verify_github_webhook_signature(secret, b"body", "no-prefix").unwrap_err();
        assert!(matches!(err, SyncError::SignatureMalformed(_)));
    }

    #[test]
    fn webhook_signature_rejects_bad_hex() {
        let err = verify_github_webhook_signature(b"s", b"b", "sha256=not-hex").unwrap_err();
        assert!(matches!(err, SyncError::SignatureMalformed(_)));
    }

    #[test]
    fn webhook_signature_rejects_wrong_secret() {
        let body = b"hello";
        let sig = compute_sig(b"right", body);
        let err = verify_github_webhook_signature(b"wrong", body, &sig).unwrap_err();
        assert!(matches!(err, SyncError::SignatureInvalid));
    }

    // ---------- issues-event branch (label-sync gap close) -----------------

    fn issue_event(action: &str, issue_node_id: &str) -> Vec<u8> {
        format!(r#"{{"action":"{action}","issue":{{"node_id":"{issue_node_id}"}}}}"#).into_bytes()
    }

    #[tokio::test]
    async fn issue_labeled_event_resyncs_linked_project_items() {
        // Item A starts in store with just channel/live, gets channel/beta
        // added — the issues.labeled event should pick up the new label
        // and reconcile the channel set without waiting for the 5-min
        // reconciler tick.
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![]);
        reader.put_single(issue("PVTI_a", "Item A", vec!["channel/live".into()]));
        let pre = mapper::map_project_item(&reader.get_project_item("PVTI_a").await.unwrap());
        apply_one(&store, &pre).await.unwrap();
        let a = store.get_item_by_slug("item-a").await.unwrap().unwrap();
        assert_eq!(store.list_channel_statuses(a.id).await.unwrap().len(), 1);

        // Update the linked item's labels and the issue→item link.
        reader.put_single(issue(
            "PVTI_a",
            "Item A",
            vec!["channel/live".into(), "channel/beta".into()],
        ));
        reader.put_issue_link("I_abc", "PVTI_a", "PVT_proj");

        let body = issue_event("labeled", "I_abc");
        handle_webhook_verified(&store, &reader, "issues", "PVT_proj", &body)
            .await
            .unwrap();
        assert_eq!(store.list_channel_statuses(a.id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn issue_unlabeled_event_archives_dropped_channels() {
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![]);
        reader.put_single(issue(
            "PVTI_a",
            "Item A",
            vec!["channel/live".into(), "channel/beta".into()],
        ));
        let pre = mapper::map_project_item(&reader.get_project_item("PVTI_a").await.unwrap());
        apply_one(&store, &pre).await.unwrap();
        let a = store.get_item_by_slug("item-a").await.unwrap().unwrap();
        assert_eq!(store.list_channel_statuses(a.id).await.unwrap().len(), 2);

        reader.put_single(issue("PVTI_a", "Item A", vec!["channel/live".into()]));
        reader.put_issue_link("I_abc", "PVTI_a", "PVT_proj");

        let body = issue_event("unlabeled", "I_abc");
        handle_webhook_verified(&store, &reader, "issues", "PVT_proj", &body)
            .await
            .unwrap();
        assert_eq!(store.list_channel_statuses(a.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn issue_opened_event_runs_initial_upsert() {
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![]);
        reader.put_single(issue("PVTI_a", "Item A", vec!["channel/live".into()]));
        reader.put_issue_link("I_abc", "PVTI_a", "PVT_proj");

        let body = issue_event("opened", "I_abc");
        handle_webhook_verified(&store, &reader, "issues", "PVT_proj", &body)
            .await
            .unwrap();
        assert!(store.get_item_by_slug("item-a").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn issue_assigned_event_silently_noops() {
        // Non-label/non-open actions never re-fetch — the receiver
        // saves the round-trip for events the mapper doesn't care
        // about (assigned, milestoned, pinned, etc.).
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![]);
        reader.put_single(issue("PVTI_a", "Item A", vec!["channel/live".into()]));
        reader.put_issue_link("I_abc", "PVTI_a", "PVT_proj");

        let body = issue_event("assigned", "I_abc");
        handle_webhook_verified(&store, &reader, "issues", "PVT_proj", &body)
            .await
            .unwrap();
        // No upsert happened — store has no row for this item.
        assert!(store.get_item_by_slug("item-a").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn issue_event_filters_to_configured_project_only() {
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![]);
        reader.put_single(issue("PVTI_a", "Item A", vec!["channel/live".into()]));
        reader.put_single(issue(
            "PVTI_other",
            "Other Project",
            vec!["channel/live".into()],
        ));
        // Issue is on TWO projects.
        reader.put_issue_link("I_abc", "PVTI_a", "PVT_proj");
        reader.put_issue_link("I_abc", "PVTI_other", "PVT_external");

        let body = issue_event("labeled", "I_abc");
        handle_webhook_verified(&store, &reader, "issues", "PVT_proj", &body)
            .await
            .unwrap();
        // Only the in-project item was upserted.
        assert!(store.get_item_by_slug("item-a").await.unwrap().is_some());
        assert!(store
            .get_item_by_slug("other-project")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn issue_event_with_no_linked_items_is_noop() {
        // An issue not on any of our Projects → empty filter result →
        // we don't crash, we don't error, we just return Ok(()).
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![]);
        let body = issue_event("labeled", "I_orphan");
        handle_webhook_verified(&store, &reader, "issues", "PVT_proj", &body)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn unknown_event_header_returns_unknown_event_error() {
        let store = MemoryRoadmapStore::new();
        let reader = FakeReader::new(vec![]);
        let err =
            handle_webhook_verified(&store, &reader, "marketplace_purchase", "PVT_proj", b"{}")
                .await
                .unwrap_err();
        match err {
            SyncError::UnknownEvent(name) => assert_eq!(name, "marketplace_purchase"),
            other => panic!("expected UnknownEvent, got {other:?}"),
        }
    }
}
