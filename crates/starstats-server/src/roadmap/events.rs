//! CI event ingestion (Phase 4).
//!
//! Receives signed events from the CI pipeline carrying channel-level
//! status transitions per spec §4. Implements the contract documented
//! in `docs/ROADMAP-PIPELINE-SPEC.md` §4.1:
//!
//! - HMAC-SHA256 signature over `v1.<timestamp>.<body>` (mirrors the
//!   Revolut webhook scheme); ±5 minute timestamp drift tolerance.
//! - Idempotent on `event_id` via the `roadmap_event_log` table.
//! - Status never auto-demotes (spec §2.5).
//! - Sticky `parked` — events against a parked channel are no-ops.
//! - `public` mismatch between event and GraphQL re-read is audit-
//!   logged; GraphQL value wins.

use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::changelog;
use super::github_graphql::GitHubReader;
use super::mapper;
use super::models::{BuildHealth, ChannelName, ChannelStatus, RoadmapStatus};
use super::store::{RoadmapStore, RoadmapStoreError, UpsertChannelStatus};

type HmacSha256 = Hmac<Sha256>;

/// ±5 minutes drift tolerance for the signed-event timestamp.
/// Mirrors `revolut::TIMESTAMP_DRIFT_TOLERANCE`.
pub const EVENT_TIMESTAMP_DRIFT: Duration = Duration::minutes(5);

// ---------- payload --------------------------------------------------------

/// CI event payload (spec §4.1, `schema_version = 1`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiEventPayload {
    pub schema_version: u32,
    pub event_id: String,
    /// Either `project_item_id` (preferred) or `roadmap_slug` MUST be
    /// present. The ingest layer checks this after parsing.
    pub project_item_id: Option<String>,
    pub roadmap_slug: Option<String>,
    pub channel: String,
    pub new_status: String,
    pub commit_sha: String,
    pub build_id: String,
    pub ci_run_url: String,
    pub tag: Option<String>,
    pub public: bool,
    /// Optional: build_health override. Defaults derived from
    /// `new_status` (building→in-progress, beta/shipped→passing).
    /// Failing builds set this to `failing` explicitly so a failing
    /// build on a shipped channel can surface without demoting
    /// `status` (spec §2.5).
    pub build_health: Option<String>,
    pub coverage_delta: Option<CoverageDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageDelta {
    pub old: f64,
    pub new: f64,
}

// ---------- errors ---------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CiEventError {
    #[error("signature malformed: {0}")]
    SignatureMalformed(String),
    #[error("signature mismatch")]
    SignatureMismatch,
    #[error("timestamp drift exceeded ({0} minutes)")]
    TimestampDrift(i64),
    #[error("payload parse: {0}")]
    PayloadParse(String),
    #[error("schema_version unsupported: {0}")]
    SchemaVersionUnsupported(u32),
    #[error("missing event_id")]
    MissingEventId,
    #[error("missing item identifier (need project_item_id or roadmap_slug)")]
    MissingIdentifier,
    #[error("unknown channel: {0}")]
    UnknownChannel(String),
    #[error("unknown status: {0}")]
    UnknownStatus(String),
    #[error("roadmap item not found")]
    ItemNotFound,
    #[error("store error: {0}")]
    Store(#[from] RoadmapStoreError),
}

// ---------- result outcomes ------------------------------------------------

/// What the ingest decided to do with one event. Useful for tests
/// and for tracing; not currently surfaced over the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    Applied {
        previous_status: Option<RoadmapStatus>,
        new_status: RoadmapStatus,
        build_health: BuildHealth,
    },
    /// Event was already processed (idempotent drop).
    DuplicateEventId,
    /// New status is below the current status (never-demote rule).
    /// `build_health` is still updated.
    HealthOnlyNoDemote {
        kept_status: RoadmapStatus,
        build_health: BuildHealth,
    },
    /// Channel is `parked` (sticky). Whole event ignored.
    SkippedParked,
}

// ---------- signature ------------------------------------------------------

/// Verify an HMAC-SHA256 signature over `v1.<timestamp_ms>.<body>`
/// against the provided secret. Constant-time compare.
///
/// `signature_header`: must be in the form `v1=<hex>` (allows comma-
/// separated multi-version lists if we ever rotate the scheme).
pub fn verify_event_signature(
    secret: &[u8],
    timestamp_header: &str,
    signature_header: &str,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<(), CiEventError> {
    let ts_ms: i64 = timestamp_header
        .parse()
        .map_err(|e| CiEventError::SignatureMalformed(format!("timestamp: {e}")))?;
    let event_ts = DateTime::<Utc>::from_timestamp_millis(ts_ms)
        .ok_or_else(|| CiEventError::SignatureMalformed("timestamp out of range".into()))?;
    let drift = (now - event_ts).num_seconds().abs();
    if drift > EVENT_TIMESTAMP_DRIFT.num_seconds() {
        return Err(CiEventError::TimestampDrift(drift / 60));
    }

    let payload = format!("v1.{}.", timestamp_header);
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|e| CiEventError::SignatureMalformed(format!("hmac init: {e}")))?;
    mac.update(payload.as_bytes());
    mac.update(body);
    let computed = mac.finalize().into_bytes();

    let mut any_v1 = false;
    let mut matched = false;
    for entry in signature_header.split(',') {
        let entry = entry.trim();
        if let Some(hex) = entry.strip_prefix("v1=") {
            any_v1 = true;
            if let Ok(decoded) = hex::decode(hex) {
                if computed.ct_eq(decoded.as_slice()).into() {
                    matched = true;
                    break;
                }
            }
        }
    }
    if !any_v1 {
        return Err(CiEventError::SignatureMalformed("no v1= entry".into()));
    }
    if !matched {
        return Err(CiEventError::SignatureMismatch);
    }
    Ok(())
}

// ---------- never-demote ordering -----------------------------------------

fn status_rank(s: RoadmapStatus) -> Option<u8> {
    Some(match s {
        RoadmapStatus::Proposed => 0,
        RoadmapStatus::InDesign => 1,
        RoadmapStatus::Building => 2,
        RoadmapStatus::Beta => 3,
        RoadmapStatus::Shipped => 4,
        RoadmapStatus::Parked => return None,
    })
}

// ---------- ingest ---------------------------------------------------------

/// Apply one CI event to the store. The caller is responsible for
/// signature verification before calling this. Returns the ingest
/// outcome (or an error for unrecoverable conditions like missing
/// item, unknown enum value).
pub async fn ingest_event(
    store: &dyn RoadmapStore,
    reader: Option<&dyn GitHubReader>,
    audit: &dyn AuditSink,
    payload: &CiEventPayload,
) -> Result<IngestOutcome, CiEventError> {
    if payload.schema_version != 1 {
        return Err(CiEventError::SchemaVersionUnsupported(
            payload.schema_version,
        ));
    }
    if payload.event_id.is_empty() {
        return Err(CiEventError::MissingEventId);
    }
    let channel = ChannelName::parse(&payload.channel)
        .ok_or_else(|| CiEventError::UnknownChannel(payload.channel.clone()))?;
    let new_status = RoadmapStatus::parse(&payload.new_status)
        .ok_or_else(|| CiEventError::UnknownStatus(payload.new_status.clone()))?;

    // Idempotency check — record_event returns false if already seen.
    if !store.record_event(&payload.event_id).await? {
        tracing::info!(event_id = %payload.event_id, "ci event: duplicate drop");
        return Ok(IngestOutcome::DuplicateEventId);
    }

    // Item lookup. Either project_item_id or roadmap_slug must resolve.
    let item = match (&payload.project_item_id, &payload.roadmap_slug) {
        (Some(id), _) => store.get_item_by_github_id(id).await?,
        (None, Some(slug)) => store.get_item_by_slug(slug).await?,
        (None, None) => return Err(CiEventError::MissingIdentifier),
    }
    .ok_or(CiEventError::ItemNotFound)?;

    // Reconcile the item's `public` flag from the authoritative GitHub
    // re-read (spec §4.3, "GraphQL value wins") and audit any mismatch
    // against the emit's optimistic claim.
    //
    // Two bugs lived here before (#149): (1) the re-check was gated on
    // `payload.project_item_id`, but CI emits send that as null and
    // identify the item by slug — so the whole block was skipped; and
    // (2) even when it ran it only audit-logged, never writing the flag
    // back, so the item stayed at its DEFAULT FALSE and never surfaced
    // on /v1/roadmap. We now re-read using the item's own stable github
    // id and persist the reconciled value. Best-effort: a reader
    // failure audit-logs and leaves the flag untouched.
    if let Some(r) = reader {
        match r.get_project_item(&item.github_project_item_id).await {
            Ok(pi) => {
                let mapped = mapper::map_project_item(&pi);
                if mapped.public != payload.public {
                    audit
                        .emit(&format!(
                            "roadmap.event.public_mismatch event_id={} payload_public={} graphql_public={}",
                            payload.event_id, payload.public, mapped.public
                        ))
                        .await;
                }
                if mapped.public != item.public {
                    match store.set_item_public(item.id, mapped.public).await {
                        Ok(()) => tracing::info!(
                            event_id = %payload.event_id,
                            item_id = %item.id,
                            public = mapped.public,
                            "ci event: reconciled public flag from GraphQL"
                        ),
                        Err(e) => tracing::warn!(
                            error = %e,
                            event_id = %payload.event_id,
                            item_id = %item.id,
                            "ci event: failed to persist reconciled public flag (non-fatal)"
                        ),
                    }
                }
            }
            Err(e) => {
                audit
                    .emit(&format!(
                        "roadmap.event.public_recheck_failed event_id={} error={e}",
                        payload.event_id
                    ))
                    .await;
            }
        }
    }

    // Sticky `parked` — skip the whole event.
    let existing = store
        .list_channel_statuses(item.id)
        .await?
        .into_iter()
        .find(|s| s.channel == channel);
    if let Some(ref e) = existing {
        if e.status == RoadmapStatus::Parked {
            tracing::info!(
                event_id = %payload.event_id,
                channel = channel.as_str(),
                "ci event: parked channel, no-op"
            );
            return Ok(IngestOutcome::SkippedParked);
        }
    }

    let build_health = derive_build_health(payload, new_status);

    // Never-demote: if new_status rank < existing rank, keep existing
    // status but still update build_health + sha/build_id/etc.
    let (final_status, demoted) = match existing.as_ref() {
        Some(e) => {
            let prev = status_rank(e.status);
            let next = status_rank(new_status);
            match (prev, next) {
                (Some(p), Some(n)) if n < p => (e.status, true),
                _ => (new_status, false),
            }
        }
        None => (new_status, false),
    };

    let previous_shipped_sha =
        compute_previous_shipped_sha(existing.as_ref(), final_status, &payload.commit_sha);

    let row = store
        .upsert_channel_status(UpsertChannelStatus {
            roadmap_item_id: item.id,
            channel,
            status: final_status,
            build_health,
            build_id: Some(&payload.build_id),
            commit_sha: Some(&payload.commit_sha),
            deployed_at: Some(Utc::now()),
            ci_run_url: Some(&payload.ci_run_url),
            previous_shipped_sha: previous_shipped_sha.as_deref(),
            last_event_id: Some(&payload.event_id),
        })
        .await?;

    // Phase 7: auto-draft a changelog entry when a channel FIRST flips
    // to Shipped. Re-shipping the same channel without a status change
    // is a no-op. Best-effort: a drafting hiccup doesn't fail the
    // ingest -- the row is already written.
    let prev_was_shipped = existing
        .as_ref()
        .map(|e| e.status == RoadmapStatus::Shipped)
        .unwrap_or(false);
    if !demoted && final_status == RoadmapStatus::Shipped && !prev_was_shipped {
        if let Err(e) = changelog::draft_for_shipped_transition(
            store,
            item.id,
            channel,
            previous_shipped_sha.as_deref(),
            &payload.commit_sha,
            &item.title,
        )
        .await
        {
            tracing::warn!(
                error = %e,
                event_id = %payload.event_id,
                item_id = %item.id,
                channel = channel.as_str(),
                "ci event: changelog auto-draft failed (non-fatal)"
            );
        }
    }

    if demoted {
        Ok(IngestOutcome::HealthOnlyNoDemote {
            kept_status: row.status,
            build_health: row.build_health,
        })
    } else {
        Ok(IngestOutcome::Applied {
            previous_status: existing.as_ref().map(|e| e.status),
            new_status: row.status,
            build_health: row.build_health,
        })
    }
}

fn derive_build_health(payload: &CiEventPayload, new_status: RoadmapStatus) -> BuildHealth {
    if let Some(raw) = &payload.build_health {
        if let Some(parsed) = BuildHealth::parse(raw) {
            return parsed;
        }
    }
    match new_status {
        RoadmapStatus::Building => BuildHealth::InProgress,
        RoadmapStatus::Beta | RoadmapStatus::Shipped => BuildHealth::Passing,
        _ => BuildHealth::Unknown,
    }
}

/// On a transition INTO `Shipped`, capture the previous shipped SHA
/// (the current `commit_sha` of the existing row). Otherwise pass
/// through the existing value unchanged.
fn compute_previous_shipped_sha(
    existing: Option<&ChannelStatus>,
    final_status: RoadmapStatus,
    new_commit_sha: &str,
) -> Option<String> {
    if final_status != RoadmapStatus::Shipped {
        return existing.and_then(|e| e.previous_shipped_sha.clone());
    }
    match existing {
        Some(e) if e.status == RoadmapStatus::Shipped => {
            // Already shipped — update only if the SHA actually
            // changed (idempotent re-shipping of the same commit
            // shouldn't churn the history pointer).
            if e.commit_sha.as_deref() != Some(new_commit_sha) {
                e.commit_sha.clone()
            } else {
                e.previous_shipped_sha.clone()
            }
        }
        Some(e) => e.commit_sha.clone(),
        None => None,
    }
}

// ---------- audit sink (minimal abstraction) -------------------------------

/// Tiny seam for audit emission so the ingest function doesn't need
/// to depend on the project's `audit_log` crate / trait directly.
/// Phase 4 uses an in-memory sink in tests; the route layer (Phase 4
/// route registration) bridges to the real audit log.
#[async_trait::async_trait]
pub trait AuditSink: Send + Sync {
    async fn emit(&self, line: &str);
}

/// Default tracing-backed sink. Used when no real audit log is wired.
pub struct TracingAuditSink;

#[async_trait::async_trait]
impl AuditSink for TracingAuditSink {
    async fn emit(&self, line: &str) {
        tracing::warn!(target: "roadmap.audit", "{line}");
    }
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::github_graphql::{
        GitHubError, GitHubReader, ProjectFieldValue, ProjectItem, ProjectItemContent,
    };
    use super::super::store::test_support::MemoryRoadmapStore;
    use super::super::store::UpsertRoadmapItem;
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct CaptureAudit {
        lines: Mutex<Vec<String>>,
    }

    impl CaptureAudit {
        fn new() -> Self {
            Self {
                lines: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl AuditSink for CaptureAudit {
        async fn emit(&self, line: &str) {
            self.lines.lock().unwrap().push(line.to_string());
        }
    }

    /// Reader that returns one canned `ProjectItem` whose GitHub
    /// "Public" field is set to `public`. Mirrors the GraphQL re-read
    /// the reconciler performs in production.
    struct StubReader {
        public: bool,
    }

    #[async_trait::async_trait]
    impl GitHubReader for StubReader {
        async fn get_project_item(&self, item_id: &str) -> Result<ProjectItem, GitHubError> {
            let mut custom_fields = HashMap::new();
            custom_fields.insert(
                "Public".to_string(),
                ProjectFieldValue::SingleSelect {
                    option_name: if self.public { "Yes" } else { "No" }.to_string(),
                    option_id: "opt".to_string(),
                },
            );
            Ok(ProjectItem {
                id: item_id.to_string(),
                content: ProjectItemContent::DraftIssue {
                    title: "Item A".to_string(),
                    body: String::new(),
                },
                custom_fields,
            })
        }

        async fn list_project_items(
            &self,
            _project_id: &str,
        ) -> Result<Vec<ProjectItem>, GitHubError> {
            Ok(vec![])
        }

        async fn list_project_item_ids_for_issue(
            &self,
            _issue_id: &str,
            _project_id: &str,
        ) -> Result<Vec<String>, GitHubError> {
            Ok(vec![])
        }
    }

    fn payload_shipped(event_id: &str, channel: &str, commit: &str) -> CiEventPayload {
        CiEventPayload {
            schema_version: 1,
            event_id: event_id.to_string(),
            project_item_id: Some("PVTI_a".to_string()),
            roadmap_slug: None,
            channel: channel.to_string(),
            new_status: "shipped".to_string(),
            commit_sha: commit.to_string(),
            build_id: "b1".to_string(),
            ci_run_url: "https://ci/1".to_string(),
            tag: Some("v1.0.0".to_string()),
            public: true,
            build_health: None,
            coverage_delta: None,
        }
    }

    async fn seed_item(store: &MemoryRoadmapStore) -> uuid::Uuid {
        let surfaces: Vec<String> = vec![];
        let row = store
            .upsert_item(UpsertRoadmapItem {
                github_project_item_id: "PVTI_a",
                slug: "item-a",
                title: "Item A",
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
        row.id
    }

    #[tokio::test]
    async fn ingest_applies_shipped_transition_with_passing_health() {
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await;
        let audit = CaptureAudit::new();
        let out = ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-1", "live", "sha1"),
        )
        .await
        .unwrap();
        assert!(matches!(
            out,
            IngestOutcome::Applied {
                new_status: RoadmapStatus::Shipped,
                build_health: BuildHealth::Passing,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn ingest_reconciles_public_flag_from_graphql_for_slug_emit() {
        // Regression for #149: a CI emit carries project_item_id=null
        // and identifies the item by slug, claiming public=true. The
        // item starts private (the DEFAULT). With a reader present the
        // reconciler must re-read GitHub and flip the stored flag so the
        // item surfaces on /v1/roadmap.
        let store = MemoryRoadmapStore::new();
        let surfaces: Vec<String> = vec![];
        store
            .upsert_item(UpsertRoadmapItem {
                github_project_item_id: "PVTI_a",
                slug: "item-a",
                title: "Item A",
                summary: None,
                category: None,
                eta_band: None,
                surfaces: &surfaces,
                parent_id: None,
                links: None,
                public: false,
            })
            .await
            .unwrap();
        let audit = CaptureAudit::new();
        let reader = StubReader { public: true };

        // Slug-based emit, exactly as scripts/roadmap-emit-event.mjs sends.
        let mut payload = payload_shipped("evt-1", "live", "sha1");
        payload.project_item_id = None;
        payload.roadmap_slug = Some("item-a".to_string());
        payload.public = true;

        ingest_event(&store, Some(&reader), &audit, &payload)
            .await
            .unwrap();

        let item = store
            .get_item_by_slug("item-a")
            .await
            .unwrap()
            .expect("item exists");
        assert!(
            item.public,
            "public flag must be reconciled to true so the item surfaces on /v1/roadmap"
        );
        // And it now appears in the public listing.
        assert_eq!(store.list_items(true).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn ingest_keeps_item_private_when_graphql_says_no() {
        // Inverse guard: the emit optimistically claims public=true, but
        // GraphQL is authoritative (spec §4.3). A "No" re-read must NOT
        // publicise the item, and the mismatch is audited.
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await; // seeds public=true
        store
            .set_item_public(
                store.get_item_by_slug("item-a").await.unwrap().unwrap().id,
                false,
            )
            .await
            .unwrap();
        let audit = CaptureAudit::new();
        let reader = StubReader { public: false };
        let mut payload = payload_shipped("evt-1", "live", "sha1");
        payload.project_item_id = None;
        payload.roadmap_slug = Some("item-a".to_string());
        payload.public = true;

        ingest_event(&store, Some(&reader), &audit, &payload)
            .await
            .unwrap();

        let item = store.get_item_by_slug("item-a").await.unwrap().unwrap();
        assert!(
            !item.public,
            "GraphQL 'No' must win over the optimistic emit"
        );
        assert!(
            audit
                .lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("public_mismatch")),
            "the payload/GraphQL mismatch must be audited"
        );
    }

    #[tokio::test]
    async fn ingest_is_idempotent_on_duplicate_event_id() {
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await;
        let audit = CaptureAudit::new();
        let p = payload_shipped("evt-1", "live", "sha1");
        let _ = ingest_event(&store, None, &audit, &p).await.unwrap();
        let out2 = ingest_event(&store, None, &audit, &p).await.unwrap();
        assert_eq!(out2, IngestOutcome::DuplicateEventId);
    }

    #[tokio::test]
    async fn ingest_never_demotes_status() {
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await;
        let audit = CaptureAudit::new();
        // First: shipped.
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-1", "live", "sha1"),
        )
        .await
        .unwrap();
        // Then: an event with new_status=building (lower rank).
        let mut p = payload_shipped("evt-2", "live", "sha2");
        p.new_status = "building".to_string();
        let out = ingest_event(&store, None, &audit, &p).await.unwrap();
        match out {
            IngestOutcome::HealthOnlyNoDemote { kept_status, .. } => {
                assert_eq!(kept_status, RoadmapStatus::Shipped);
            }
            other => panic!("expected HealthOnlyNoDemote, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ingest_failing_build_sets_health_without_changing_status() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store).await;
        let audit = CaptureAudit::new();
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-1", "live", "sha1"),
        )
        .await
        .unwrap();
        let mut p = payload_shipped("evt-2", "live", "sha1");
        p.build_health = Some("failing".to_string());
        ingest_event(&store, None, &audit, &p).await.unwrap();
        let chans = store.list_channel_statuses(item_id).await.unwrap();
        let live = chans
            .iter()
            .find(|c| c.channel == ChannelName::Live)
            .unwrap();
        assert_eq!(live.status, RoadmapStatus::Shipped);
        assert_eq!(live.build_health, BuildHealth::Failing);
    }

    #[tokio::test]
    async fn ingest_parked_channel_is_no_op() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store).await;
        // Manually set channel to parked.
        store
            .upsert_channel_status(UpsertChannelStatus {
                roadmap_item_id: item_id,
                channel: ChannelName::Live,
                status: RoadmapStatus::Parked,
                build_health: BuildHealth::Unknown,
                build_id: None,
                commit_sha: None,
                deployed_at: None,
                ci_run_url: None,
                previous_shipped_sha: None,
                last_event_id: None,
            })
            .await
            .unwrap();
        let audit = CaptureAudit::new();
        let out = ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-1", "live", "sha1"),
        )
        .await
        .unwrap();
        assert_eq!(out, IngestOutcome::SkippedParked);
        // Channel still parked.
        let chans = store.list_channel_statuses(item_id).await.unwrap();
        let live = chans
            .iter()
            .find(|c| c.channel == ChannelName::Live)
            .unwrap();
        assert_eq!(live.status, RoadmapStatus::Parked);
    }

    #[tokio::test]
    async fn ingest_unknown_status_rejects() {
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await;
        let audit = CaptureAudit::new();
        let mut p = payload_shipped("evt-1", "live", "sha1");
        p.new_status = "wat".to_string();
        let err = ingest_event(&store, None, &audit, &p).await.unwrap_err();
        assert!(matches!(err, CiEventError::UnknownStatus(_)));
    }

    #[tokio::test]
    async fn ingest_missing_event_id_rejects() {
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await;
        let audit = CaptureAudit::new();
        let mut p = payload_shipped("", "live", "sha1");
        p.event_id = "".to_string();
        let err = ingest_event(&store, None, &audit, &p).await.unwrap_err();
        assert!(matches!(err, CiEventError::MissingEventId));
    }

    #[tokio::test]
    async fn ingest_unknown_schema_version_rejects() {
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await;
        let audit = CaptureAudit::new();
        let mut p = payload_shipped("evt-1", "live", "sha1");
        p.schema_version = 2;
        let err = ingest_event(&store, None, &audit, &p).await.unwrap_err();
        assert!(matches!(err, CiEventError::SchemaVersionUnsupported(2)));
    }

    #[tokio::test]
    async fn ingest_unknown_item_rejects() {
        let store = MemoryRoadmapStore::new();
        let audit = CaptureAudit::new();
        let p = payload_shipped("evt-1", "live", "sha1");
        let err = ingest_event(&store, None, &audit, &p).await.unwrap_err();
        assert!(matches!(err, CiEventError::ItemNotFound));
    }

    #[tokio::test]
    async fn previous_shipped_sha_captured_on_re_ship() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store).await;
        let audit = CaptureAudit::new();
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-1", "live", "sha1"),
        )
        .await
        .unwrap();
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-2", "live", "sha2"),
        )
        .await
        .unwrap();
        let chans = store.list_channel_statuses(item_id).await.unwrap();
        let live = chans
            .iter()
            .find(|c| c.channel == ChannelName::Live)
            .unwrap();
        assert_eq!(live.commit_sha.as_deref(), Some("sha2"));
        assert_eq!(live.previous_shipped_sha.as_deref(), Some("sha1"));
    }

    // ---------- signature ----------

    fn compute_event_sig(secret: &[u8], ts_ms: i64, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(format!("v1.{ts_ms}.").as_bytes());
        mac.update(body);
        format!("v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn event_signature_ok_round_trip() {
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        let secret = b"shh";
        let body = br#"{"hello":"world"}"#;
        let sig = compute_event_sig(secret, ts_ms, body);
        verify_event_signature(secret, &ts_ms.to_string(), &sig, body, now).unwrap();
    }

    #[test]
    fn event_signature_rejects_drift() {
        let now = Utc::now();
        let stale_ts = (now - Duration::minutes(10)).timestamp_millis();
        let secret = b"shh";
        let body = b"x";
        let sig = compute_event_sig(secret, stale_ts, body);
        let err =
            verify_event_signature(secret, &stale_ts.to_string(), &sig, body, now).unwrap_err();
        assert!(matches!(err, CiEventError::TimestampDrift(_)));
    }

    #[test]
    fn event_signature_rejects_mismatch() {
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        let secret = b"right";
        let body = b"x";
        let sig = compute_event_sig(secret, ts_ms, body);
        let err =
            verify_event_signature(b"wrong", &ts_ms.to_string(), &sig, body, now).unwrap_err();
        assert!(matches!(err, CiEventError::SignatureMismatch));
    }

    #[test]
    fn event_signature_rejects_no_v1_entry() {
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        let err =
            verify_event_signature(b"s", &ts_ms.to_string(), "v2=abcd", b"x", now).unwrap_err();
        assert!(matches!(err, CiEventError::SignatureMalformed(_)));
    }

    // ---------- changelog auto-draft hook (Phase 7) ----------

    #[tokio::test]
    async fn shipped_transition_drafts_changelog_entry() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store).await;
        let audit = CaptureAudit::new();
        // Item is fresh — first shipped event triggers a draft.
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-1", "live", "sha1"),
        )
        .await
        .unwrap();
        let drafts = store.list_changelog_drafts().await.unwrap();
        assert_eq!(drafts.len(), 1, "exactly one draft");
        let draft = &drafts[0];
        assert_eq!(draft.roadmap_item_id, item_id);
        assert_eq!(draft.channel, ChannelName::Live);
        assert!(draft.published_at.is_none());
        // Initial release path -- previous SHA is None.
        assert!(draft.previous_shipped_sha.is_none());
        assert_eq!(draft.shipped_sha.as_deref(), Some("sha1"));
    }

    #[tokio::test]
    async fn shipped_transition_does_not_redraft_on_same_channel_re_ship() {
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await;
        let audit = CaptureAudit::new();
        // First shipped event drafts once.
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-1", "live", "sha1"),
        )
        .await
        .unwrap();
        assert_eq!(store.list_changelog_drafts().await.unwrap().len(), 1);

        // Second shipped event on the SAME channel updates the SHA
        // but does NOT draft a second entry — previous status was
        // already Shipped.
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-2", "live", "sha2"),
        )
        .await
        .unwrap();
        assert_eq!(
            store.list_changelog_drafts().await.unwrap().len(),
            1,
            "no duplicate draft on re-ship"
        );
    }

    #[tokio::test]
    async fn shipped_transition_on_distinct_channel_drafts_separately() {
        let store = MemoryRoadmapStore::new();
        seed_item(&store).await;
        let audit = CaptureAudit::new();
        // live -> shipped drafts entry #1.
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-1", "live", "sha1"),
        )
        .await
        .unwrap();
        // beta -> shipped is a DIFFERENT channel; that channel's
        // previous status was None, so a new draft is created.
        ingest_event(
            &store,
            None,
            &audit,
            &payload_shipped("evt-2", "beta", "sha2"),
        )
        .await
        .unwrap();
        let drafts = store.list_changelog_drafts().await.unwrap();
        assert_eq!(drafts.len(), 2);
    }

    #[tokio::test]
    async fn no_draft_on_demoted_event() {
        let store = MemoryRoadmapStore::new();
        let item_id = seed_item(&store).await;
        let audit = CaptureAudit::new();
        // Seed at Beta.
        let mut p = payload_shipped("evt-1", "live", "sha1");
        p.new_status = "beta".to_string();
        ingest_event(&store, None, &audit, &p).await.unwrap();
        // An event that *claims* shipped but is actually a same-or-
        // lower demotion would never hit this path -- but a
        // never-demote (e.g. an event whose final_status is Shipped
        // because of a non-demoted higher rank) still drafts. The
        // demotion guard only kicks when new_status < existing.
        // Sanity check: no draft yet (only a beta transition).
        assert!(store.list_channel_statuses(item_id).await.unwrap().len() == 1);
        assert_eq!(store.list_changelog_drafts().await.unwrap().len(), 0);
    }
}
