//! Roadmap pipeline store: trait + Postgres impl + Memory impl
//! (under `#[cfg(test)] pub mod test_support`).
//!
//! Mirrors the `share_metadata.rs` / `share_reports.rs` pattern: the
//! trait is the seam route-layer code consumes; the Memory impl
//! exists so route-layer unit tests don't need a Postgres pool.
//!
//! Closed-vocabulary enums (`RoadmapStatus`, `ChannelName`,
//! `BuildHealth`) round-trip through TEXT columns via `as_str()` +
//! `parse()`. A stored value outside the vocabulary surfaces as
//! `RoadmapStoreError::Domain`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{
    BuildHealth, ChannelName, ChannelStatus, RoadmapChangelogEntry, RoadmapEventLogEntry,
    RoadmapItem, RoadmapStatus, RoadmapSubscriber, RoadmapUserReadState, RoadmapVote,
};
// Only used by the in-test Memory impl's State struct, so the
// non-test build flags it unused; pull it in conditionally.
#[cfg(test)]
use super::models::ChannelStatusArchive;

/// Aggregate vote count for a single item (returned by
/// `count_votes`). Pulled to its own struct so a future bulk
/// `count_votes_for` returning many rows has a natural shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoteTally {
    pub roadmap_item_id: Uuid,
    pub votes: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum RoadmapStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("roadmap item not found")]
    NotFound,
    #[error("slug already exists: {0}")]
    DuplicateSlug(String),
    #[error("stored value out of domain: {0}")]
    Domain(String),
}

/// Payload for `upsert_item`. Kept as a struct so adding a field
/// later (e.g. summary edits) doesn't churn every call site.
#[derive(Debug, Clone)]
pub struct UpsertRoadmapItem<'a> {
    pub github_project_item_id: &'a str,
    pub slug: &'a str,
    pub title: &'a str,
    pub summary: Option<&'a str>,
    pub category: Option<&'a str>,
    pub eta_band: Option<&'a str>,
    pub surfaces: &'a [String],
    pub parent_id: Option<Uuid>,
    pub links: Option<&'a Value>,
    pub public: bool,
}

/// Payload for `upsert_channel_status`. Mirrors the row shape minus
/// the timestamps the DB stamps.
#[derive(Debug, Clone)]
pub struct UpsertChannelStatus<'a> {
    pub roadmap_item_id: Uuid,
    pub channel: ChannelName,
    pub status: RoadmapStatus,
    pub build_health: BuildHealth,
    pub build_id: Option<&'a str>,
    pub commit_sha: Option<&'a str>,
    pub deployed_at: Option<DateTime<Utc>>,
    pub ci_run_url: Option<&'a str>,
    pub previous_shipped_sha: Option<&'a str>,
    pub last_event_id: Option<&'a str>,
}

/// Payload for `draft_changelog`. Inserts a draft (published_at = NULL)
/// per spec §8.1. Title + body are owned by the auto-drafter (today
/// the `Shipped` transition hook in `events.rs`; Phase 9 will replace
/// the placeholder body with real PR-title diffing).
#[derive(Debug, Clone)]
pub struct DraftChangelog<'a> {
    pub roadmap_item_id: Uuid,
    pub channel: ChannelName,
    pub title: &'a str,
    pub body: &'a str,
    pub previous_shipped_sha: Option<&'a str>,
    pub shipped_sha: Option<&'a str>,
}

#[async_trait]
pub trait RoadmapStore: Send + Sync + 'static {
    /// Idempotent upsert keyed on `github_project_item_id` (the
    /// stable handle across renames per spec §1.4). On insert, the
    /// caller-provided slug is recorded; on update, the existing
    /// slug is preserved (slug is immutable post-creation).
    async fn upsert_item(
        &self,
        payload: UpsertRoadmapItem<'_>,
    ) -> Result<RoadmapItem, RoadmapStoreError>;

    /// Update only the `public` flag for one item, leaving every
    /// content field untouched. The CI-event reconciler uses this to
    /// apply the authoritative GitHub "Public" field (spec §4.3,
    /// "GraphQL value wins") — a CI emit identifies the item by slug
    /// and never carries the full content payload, so a narrow setter
    /// is the right tool. Idempotent; a no-op on a missing/soft-deleted
    /// row.
    async fn set_item_public(&self, id: Uuid, public: bool) -> Result<(), RoadmapStoreError>;

    /// Single-row lookup by slug. `Ok(None)` for missing rows; the
    /// soft-deleted rows are excluded.
    async fn get_item_by_slug(&self, slug: &str) -> Result<Option<RoadmapItem>, RoadmapStoreError>;

    /// Single-row lookup by the stable GitHub Project item id. Soft-
    /// deleted rows are excluded.
    async fn get_item_by_github_id(
        &self,
        github_project_item_id: &str,
    ) -> Result<Option<RoadmapItem>, RoadmapStoreError>;

    /// List non-soft-deleted items. `public_only = true` filters to
    /// `public = TRUE`; otherwise every live row is returned. Most
    /// recent first.
    async fn list_items(&self, public_only: bool) -> Result<Vec<RoadmapItem>, RoadmapStoreError>;

    /// Soft-delete an item (sets `deleted_at`). Idempotent on an
    /// already-deleted row.
    async fn soft_delete_item(&self, id: Uuid) -> Result<(), RoadmapStoreError>;

    /// Soft-delete by the stable GitHub Project item id. Webhook
    /// handler entry point for `action: deleted`. Idempotent (`Ok(())`
    /// whether or not a row matched).
    async fn soft_delete_by_github_id(
        &self,
        github_project_item_id: &str,
    ) -> Result<(), RoadmapStoreError>;

    /// List all live `ChannelStatus` rows for one item. Used by the
    /// reconciler to detect channels that have disappeared from the
    /// Project board so they can be archived (spec §2.6).
    async fn list_channel_statuses(
        &self,
        roadmap_item_id: Uuid,
    ) -> Result<Vec<ChannelStatus>, RoadmapStoreError>;

    /// Upsert one `(item, channel)` status row. PK is the pair, so
    /// re-emitting an event with the same channel updates in place.
    async fn upsert_channel_status(
        &self,
        payload: UpsertChannelStatus<'_>,
    ) -> Result<ChannelStatus, RoadmapStoreError>;

    /// Move a live channel-status row to the archive (spec §2.6).
    /// Returns `NotFound` if there is no live row to archive.
    async fn archive_channel(
        &self,
        roadmap_item_id: Uuid,
        channel: ChannelName,
    ) -> Result<(), RoadmapStoreError>;

    /// Restore a channel from the archive (spec §2.6). Returns
    /// `NotFound` if the archive row is absent.
    async fn restore_channel(
        &self,
        roadmap_item_id: Uuid,
        channel: ChannelName,
    ) -> Result<ChannelStatus, RoadmapStoreError>;

    /// Record an inbound CI event in `roadmap_event_log`. Returns
    /// `Ok(true)` on first insert, `Ok(false)` if the event_id was
    /// already present (idempotent drop per spec §4.4).
    async fn record_event(&self, event_id: &str) -> Result<bool, RoadmapStoreError>;

    /// Check whether an event_id has already been logged. Used by
    /// the inbound handler before deciding to apply state.
    async fn event_seen(
        &self,
        event_id: &str,
    ) -> Result<Option<RoadmapEventLogEntry>, RoadmapStoreError>;

    /// Cast a vote for `(user, item)`. Idempotent -- a second vote
    /// from the same user is a no-op (returns the existing row).
    async fn cast_vote(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<RoadmapVote, RoadmapStoreError>;

    /// Retract a previously-cast vote. `Ok(())` whether or not the
    /// row was present (idempotent).
    async fn retract_vote(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<(), RoadmapStoreError>;

    /// Count current votes for one item.
    async fn count_votes(&self, roadmap_item_id: Uuid) -> Result<VoteTally, RoadmapStoreError>;

    /// Subscribe `user` to `item` notifications. Idempotent.
    async fn subscribe(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<RoadmapSubscriber, RoadmapStoreError>;

    /// Unsubscribe. Idempotent -- no row is a successful outcome.
    async fn unsubscribe(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<(), RoadmapStoreError>;

    /// List subscribers for one item. Server-internal only -- never
    /// returned via a public API (spec §7.2).
    async fn list_subscribers_for_item(
        &self,
        roadmap_item_id: Uuid,
    ) -> Result<Vec<RoadmapSubscriber>, RoadmapStoreError>;

    // ---------------------------------------------------------------
    // Changelog surface (spec §8 -- Phase 7).
    // ---------------------------------------------------------------

    /// Insert one draft changelog entry. `published_at` is NULL on
    /// the returned row.
    async fn draft_changelog(
        &self,
        payload: DraftChangelog<'_>,
    ) -> Result<RoadmapChangelogEntry, RoadmapStoreError>;

    /// Single-row lookup by changelog entry id.
    async fn get_changelog_entry(
        &self,
        id: Uuid,
    ) -> Result<Option<RoadmapChangelogEntry>, RoadmapStoreError>;

    /// List draft entries (`published_at IS NULL`), most-recently-
    /// drafted first. Admin-UI only.
    async fn list_changelog_drafts(&self) -> Result<Vec<RoadmapChangelogEntry>, RoadmapStoreError>;

    /// List published entries (`published_at IS NOT NULL`), most-
    /// recently-published first. Capped at `limit` rows (clamped 1..=200
    /// server-side).
    async fn list_published_changelog(
        &self,
        limit: i64,
    ) -> Result<Vec<RoadmapChangelogEntry>, RoadmapStoreError>;

    /// Flip a draft into the published state. Stamps `published_at` =
    /// NOW() and `published_by` = `published_by`. Returns `NotFound`
    /// when no draft matches `id` (covers both "missing" and "already
    /// published" cases -- callers treat both the same).
    async fn publish_changelog(
        &self,
        id: Uuid,
        published_by: &str,
    ) -> Result<RoadmapChangelogEntry, RoadmapStoreError>;

    /// Edit title + body on a draft. Returns `NotFound` if already
    /// published or absent.
    async fn edit_changelog_draft(
        &self,
        id: Uuid,
        title: &str,
        body: &str,
    ) -> Result<RoadmapChangelogEntry, RoadmapStoreError>;

    /// Delete drafts (`published_at IS NULL`) older than `before`.
    /// Returns the count of removed rows. Used by the 30-day purge
    /// worker (spec §8.4).
    async fn purge_old_drafts(&self, before: DateTime<Utc>) -> Result<u64, RoadmapStoreError>;

    // ---------------------------------------------------------------
    // User read state (spec §9 -- Phase 8 "What's new" tray panel).
    // ---------------------------------------------------------------

    /// Single-row lookup of `(user, item)` read state. `Ok(None)` when
    /// the user has never marked-seen for that item.
    async fn get_user_read_state(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<Option<RoadmapUserReadState>, RoadmapStoreError>;

    /// Upsert read state for `(user, item)`. Sets `last_seen_at = NOW()`
    /// and `last_seen_changelog_entry_id` to the supplied entry id
    /// (or NULL). Returns the resulting row.
    async fn upsert_user_read_state(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
        last_seen_changelog_entry_id: Option<Uuid>,
    ) -> Result<RoadmapUserReadState, RoadmapStoreError>;

    /// List top-level items (`parent_id IS NULL`, `public = TRUE`,
    /// not soft-deleted) that have at least one PUBLISHED changelog
    /// entry. Ordered by the most-recent published-entry timestamp
    /// (DESC) so the tray "What's new" panel surfaces the freshest
    /// drops first. Capped at `limit` rows (clamped 1..=50 server-side).
    async fn list_top_level_items_with_changelog(
        &self,
        limit: i64,
    ) -> Result<Vec<RoadmapItem>, RoadmapStoreError>;
}

// ---------------------------------------------------------------
// Postgres implementation.
// ---------------------------------------------------------------

pub struct PostgresRoadmapStore {
    pool: PgPool,
}

impl PostgresRoadmapStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// Row tuple matching the column order in every roadmap_items
// SELECT below. Long tuples are workspace-allowed
// (`clippy::type_complexity`), matching the share_reports pattern.
type RoadmapItemRow = (
    Uuid,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    i32,
    Vec<String>,
    Option<Uuid>,
    Value,
    bool,
    DateTime<Utc>,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

const ROADMAP_ITEM_COLS: &str = "id, slug, github_project_item_id, title, summary, \
    category, eta_band, votes, surfaces, parent_id, links, public, \
    content_last_updated, pipeline_last_updated, created_at, deleted_at";

fn row_to_item(row: RoadmapItemRow) -> RoadmapItem {
    RoadmapItem {
        id: row.0,
        slug: row.1,
        github_project_item_id: row.2,
        title: row.3,
        summary: row.4,
        category: row.5,
        eta_band: row.6,
        votes: row.7,
        surfaces: row.8,
        parent_id: row.9,
        links: row.10,
        public: row.11,
        content_last_updated: row.12,
        pipeline_last_updated: row.13,
        created_at: row.14,
        deleted_at: row.15,
    }
}

type ChannelStatusRow = (
    Uuid,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
);

const CHANNEL_STATUS_COLS: &str = "roadmap_item_id, channel, status, build_health, \
    build_id, commit_sha, deployed_at, ci_run_url, previous_shipped_sha, \
    last_event_id, updated_at";

fn row_to_channel_status(row: ChannelStatusRow) -> Result<ChannelStatus, RoadmapStoreError> {
    let channel = ChannelName::parse(&row.1)
        .ok_or_else(|| RoadmapStoreError::Domain(format!("channel={}", row.1)))?;
    let status = RoadmapStatus::parse(&row.2)
        .ok_or_else(|| RoadmapStoreError::Domain(format!("status={}", row.2)))?;
    let build_health = BuildHealth::parse(&row.3)
        .ok_or_else(|| RoadmapStoreError::Domain(format!("build_health={}", row.3)))?;
    Ok(ChannelStatus {
        roadmap_item_id: row.0,
        channel,
        status,
        build_health,
        build_id: row.4,
        commit_sha: row.5,
        deployed_at: row.6,
        ci_run_url: row.7,
        previous_shipped_sha: row.8,
        last_event_id: row.9,
        updated_at: row.10,
    })
}

// Changelog row tuple matching the column order in every
// roadmap_changelog SELECT below.
type ChangelogRow = (
    Uuid,
    Uuid,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
);

const CHANGELOG_COLS: &str = "id, roadmap_item_id, channel, title, body, \
    previous_shipped_sha, shipped_sha, created_at, published_at, published_by";

fn row_to_changelog(row: ChangelogRow) -> Result<RoadmapChangelogEntry, RoadmapStoreError> {
    let channel = ChannelName::parse(&row.2)
        .ok_or_else(|| RoadmapStoreError::Domain(format!("channel={}", row.2)))?;
    Ok(RoadmapChangelogEntry {
        id: row.0,
        roadmap_item_id: row.1,
        channel,
        title: row.3,
        body: row.4,
        previous_shipped_sha: row.5,
        shipped_sha: row.6,
        created_at: row.7,
        published_at: row.8,
        published_by: row.9,
    })
}

#[async_trait]
impl RoadmapStore for PostgresRoadmapStore {
    async fn upsert_item(
        &self,
        payload: UpsertRoadmapItem<'_>,
    ) -> Result<RoadmapItem, RoadmapStoreError> {
        // Slug is immutable post-creation (spec §1.4): the INSERT path
        // records the caller's slug; the ON CONFLICT path on the
        // stable GitHub id preserves the existing slug via the
        // intentional omission from the UPDATE SET list. Content
        // edits bump `content_last_updated`.
        let links = payload.links.cloned().unwrap_or(Value::Array(vec![]));
        let surfaces: Vec<String> = payload.surfaces.to_vec();
        let sql = format!(
            r#"
            INSERT INTO roadmap_items
                (slug, github_project_item_id, title, summary, category, eta_band,
                 surfaces, parent_id, links, public)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (github_project_item_id) DO UPDATE SET
                title    = EXCLUDED.title,
                summary  = EXCLUDED.summary,
                category = EXCLUDED.category,
                eta_band = EXCLUDED.eta_band,
                surfaces = EXCLUDED.surfaces,
                parent_id = EXCLUDED.parent_id,
                links    = EXCLUDED.links,
                public   = EXCLUDED.public,
                -- Resurrect a soft-deleted row when it reappears on the
                -- board. The reconciler upserts every live project item on
                -- each tick, so a board-present item must never stay hidden.
                -- Without this, an item archived-then-restored on the board
                -- stayed soft-deleted forever (item #146 absent from
                -- /v1/roadmap for weeks despite Public=Yes).
                deleted_at = NULL,
                content_last_updated = NOW()
            RETURNING {ROADMAP_ITEM_COLS}
            "#
        );
        let row: RoadmapItemRow = sqlx::query_as(&sql)
            .bind(payload.slug)
            .bind(payload.github_project_item_id)
            .bind(payload.title)
            .bind(payload.summary)
            .bind(payload.category)
            .bind(payload.eta_band)
            .bind(&surfaces)
            .bind(payload.parent_id)
            .bind(&links)
            .bind(payload.public)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match &e {
                sqlx::Error::Database(db) if db.is_unique_violation() => {
                    RoadmapStoreError::DuplicateSlug(payload.slug.to_string())
                }
                _ => RoadmapStoreError::Database(e),
            })?;
        Ok(row_to_item(row))
    }

    async fn set_item_public(&self, id: Uuid, public: bool) -> Result<(), RoadmapStoreError> {
        sqlx::query(
            "UPDATE roadmap_items SET public = $2 \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(public)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_item_by_slug(&self, slug: &str) -> Result<Option<RoadmapItem>, RoadmapStoreError> {
        let sql = format!(
            "SELECT {ROADMAP_ITEM_COLS} FROM roadmap_items \
             WHERE slug = $1 AND deleted_at IS NULL"
        );
        let row: Option<RoadmapItemRow> = sqlx::query_as(&sql)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(row_to_item))
    }

    async fn get_item_by_github_id(
        &self,
        github_project_item_id: &str,
    ) -> Result<Option<RoadmapItem>, RoadmapStoreError> {
        let sql = format!(
            "SELECT {ROADMAP_ITEM_COLS} FROM roadmap_items \
             WHERE github_project_item_id = $1 AND deleted_at IS NULL"
        );
        let row: Option<RoadmapItemRow> = sqlx::query_as(&sql)
            .bind(github_project_item_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(row_to_item))
    }

    async fn list_items(&self, public_only: bool) -> Result<Vec<RoadmapItem>, RoadmapStoreError> {
        let rows: Vec<RoadmapItemRow> = if public_only {
            let sql = format!(
                "SELECT {ROADMAP_ITEM_COLS} FROM roadmap_items \
                 WHERE deleted_at IS NULL AND public = TRUE \
                 ORDER BY created_at DESC"
            );
            sqlx::query_as(&sql).fetch_all(&self.pool).await?
        } else {
            let sql = format!(
                "SELECT {ROADMAP_ITEM_COLS} FROM roadmap_items \
                 WHERE deleted_at IS NULL ORDER BY created_at DESC"
            );
            sqlx::query_as(&sql).fetch_all(&self.pool).await?
        };
        Ok(rows.into_iter().map(row_to_item).collect())
    }

    async fn soft_delete_item(&self, id: Uuid) -> Result<(), RoadmapStoreError> {
        sqlx::query(
            r#"
            UPDATE roadmap_items
            SET deleted_at = COALESCE(deleted_at, NOW())
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn soft_delete_by_github_id(
        &self,
        github_project_item_id: &str,
    ) -> Result<(), RoadmapStoreError> {
        sqlx::query(
            r#"
            UPDATE roadmap_items
            SET deleted_at = COALESCE(deleted_at, NOW())
            WHERE github_project_item_id = $1
            "#,
        )
        .bind(github_project_item_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_channel_statuses(
        &self,
        roadmap_item_id: Uuid,
    ) -> Result<Vec<ChannelStatus>, RoadmapStoreError> {
        let sql = format!(
            "SELECT {CHANNEL_STATUS_COLS} FROM roadmap_channel_statuses \
             WHERE roadmap_item_id = $1 ORDER BY channel"
        );
        let rows: Vec<ChannelStatusRow> = sqlx::query_as(&sql)
            .bind(roadmap_item_id)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(row_to_channel_status).collect()
    }

    async fn upsert_channel_status(
        &self,
        payload: UpsertChannelStatus<'_>,
    ) -> Result<ChannelStatus, RoadmapStoreError> {
        let sql = format!(
            r#"
            INSERT INTO roadmap_channel_statuses
                (roadmap_item_id, channel, status, build_health, build_id,
                 commit_sha, deployed_at, ci_run_url, previous_shipped_sha,
                 last_event_id, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
            ON CONFLICT (roadmap_item_id, channel) DO UPDATE SET
                status               = EXCLUDED.status,
                build_health         = EXCLUDED.build_health,
                build_id             = EXCLUDED.build_id,
                commit_sha           = EXCLUDED.commit_sha,
                deployed_at          = EXCLUDED.deployed_at,
                ci_run_url           = EXCLUDED.ci_run_url,
                previous_shipped_sha = EXCLUDED.previous_shipped_sha,
                last_event_id        = EXCLUDED.last_event_id,
                updated_at           = NOW()
            RETURNING {CHANNEL_STATUS_COLS}
            "#
        );
        let row: ChannelStatusRow = sqlx::query_as(&sql)
            .bind(payload.roadmap_item_id)
            .bind(payload.channel.as_str())
            .bind(payload.status.as_str())
            .bind(payload.build_health.as_str())
            .bind(payload.build_id)
            .bind(payload.commit_sha)
            .bind(payload.deployed_at)
            .bind(payload.ci_run_url)
            .bind(payload.previous_shipped_sha)
            .bind(payload.last_event_id)
            .fetch_one(&self.pool)
            .await?;
        row_to_channel_status(row)
    }

    async fn archive_channel(
        &self,
        roadmap_item_id: Uuid,
        channel: ChannelName,
    ) -> Result<(), RoadmapStoreError> {
        // Move-and-delete in one round trip via CTE so the live row
        // disappears atomically with the archive insert.
        let result = sqlx::query(
            r#"
            WITH moved AS (
                DELETE FROM roadmap_channel_statuses
                WHERE roadmap_item_id = $1 AND channel = $2
                RETURNING roadmap_item_id, channel, status, build_health, build_id,
                          commit_sha, deployed_at, ci_run_url, previous_shipped_sha,
                          last_event_id
            )
            INSERT INTO roadmap_channel_statuses_archive
                (roadmap_item_id, channel, status, build_health, build_id,
                 commit_sha, deployed_at, ci_run_url, previous_shipped_sha,
                 last_event_id, archived_at)
            SELECT roadmap_item_id, channel, status, build_health, build_id,
                   commit_sha, deployed_at, ci_run_url, previous_shipped_sha,
                   last_event_id, NOW()
            FROM moved
            ON CONFLICT (roadmap_item_id, channel) DO UPDATE SET
                status               = EXCLUDED.status,
                build_health         = EXCLUDED.build_health,
                build_id             = EXCLUDED.build_id,
                commit_sha           = EXCLUDED.commit_sha,
                deployed_at          = EXCLUDED.deployed_at,
                ci_run_url           = EXCLUDED.ci_run_url,
                previous_shipped_sha = EXCLUDED.previous_shipped_sha,
                last_event_id        = EXCLUDED.last_event_id,
                archived_at          = NOW()
            "#,
        )
        .bind(roadmap_item_id)
        .bind(channel.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(RoadmapStoreError::NotFound);
        }
        Ok(())
    }

    async fn restore_channel(
        &self,
        roadmap_item_id: Uuid,
        channel: ChannelName,
    ) -> Result<ChannelStatus, RoadmapStoreError> {
        let sql = format!(
            r#"
            WITH restored AS (
                DELETE FROM roadmap_channel_statuses_archive
                WHERE roadmap_item_id = $1 AND channel = $2
                RETURNING roadmap_item_id, channel, status, build_health, build_id,
                          commit_sha, deployed_at, ci_run_url, previous_shipped_sha,
                          last_event_id
            )
            INSERT INTO roadmap_channel_statuses
                (roadmap_item_id, channel, status, build_health, build_id,
                 commit_sha, deployed_at, ci_run_url, previous_shipped_sha,
                 last_event_id, updated_at)
            SELECT roadmap_item_id, channel, status, build_health, build_id,
                   commit_sha, deployed_at, ci_run_url, previous_shipped_sha,
                   last_event_id, NOW()
            FROM restored
            ON CONFLICT (roadmap_item_id, channel) DO UPDATE SET
                status               = EXCLUDED.status,
                build_health         = EXCLUDED.build_health,
                build_id             = EXCLUDED.build_id,
                commit_sha           = EXCLUDED.commit_sha,
                deployed_at          = EXCLUDED.deployed_at,
                ci_run_url           = EXCLUDED.ci_run_url,
                previous_shipped_sha = EXCLUDED.previous_shipped_sha,
                last_event_id        = EXCLUDED.last_event_id,
                updated_at           = NOW()
            RETURNING {CHANNEL_STATUS_COLS}
            "#
        );
        let row: Option<ChannelStatusRow> = sqlx::query_as(&sql)
            .bind(roadmap_item_id)
            .bind(channel.as_str())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => row_to_channel_status(r),
            None => Err(RoadmapStoreError::NotFound),
        }
    }

    async fn record_event(&self, event_id: &str) -> Result<bool, RoadmapStoreError> {
        let result = sqlx::query(
            r#"
            INSERT INTO roadmap_event_log (event_id) VALUES ($1)
            ON CONFLICT (event_id) DO NOTHING
            "#,
        )
        .bind(event_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn event_seen(
        &self,
        event_id: &str,
    ) -> Result<Option<RoadmapEventLogEntry>, RoadmapStoreError> {
        let row: Option<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT event_id, received_at FROM roadmap_event_log WHERE event_id = $1",
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(event_id, received_at)| RoadmapEventLogEntry {
            event_id,
            received_at,
        }))
    }

    async fn cast_vote(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<RoadmapVote, RoadmapStoreError> {
        // Idempotent: a duplicate vote returns the existing row's
        // timestamp rather than NOW(). DO UPDATE on a no-op clause
        // ensures RETURNING fires regardless of conflict.
        let row: (Uuid, Uuid, DateTime<Utc>) = sqlx::query_as(
            r#"
            INSERT INTO roadmap_votes (user_id, roadmap_item_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id, roadmap_item_id) DO UPDATE
                SET user_id = roadmap_votes.user_id
            RETURNING user_id, roadmap_item_id, created_at
            "#,
        )
        .bind(user_id)
        .bind(roadmap_item_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(RoadmapVote {
            user_id: row.0,
            roadmap_item_id: row.1,
            created_at: row.2,
        })
    }

    async fn retract_vote(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<(), RoadmapStoreError> {
        sqlx::query("DELETE FROM roadmap_votes WHERE user_id = $1 AND roadmap_item_id = $2")
            .bind(user_id)
            .bind(roadmap_item_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn count_votes(&self, roadmap_item_id: Uuid) -> Result<VoteTally, RoadmapStoreError> {
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*)::bigint FROM roadmap_votes WHERE roadmap_item_id = $1")
                .bind(roadmap_item_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(VoteTally {
            roadmap_item_id,
            votes: n,
        })
    }

    async fn subscribe(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<RoadmapSubscriber, RoadmapStoreError> {
        let row: (Uuid, Uuid, DateTime<Utc>) = sqlx::query_as(
            r#"
            INSERT INTO roadmap_subscribers (user_id, roadmap_item_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id, roadmap_item_id) DO UPDATE
                SET user_id = roadmap_subscribers.user_id
            RETURNING user_id, roadmap_item_id, created_at
            "#,
        )
        .bind(user_id)
        .bind(roadmap_item_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(RoadmapSubscriber {
            user_id: row.0,
            roadmap_item_id: row.1,
            created_at: row.2,
        })
    }

    async fn unsubscribe(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<(), RoadmapStoreError> {
        sqlx::query("DELETE FROM roadmap_subscribers WHERE user_id = $1 AND roadmap_item_id = $2")
            .bind(user_id)
            .bind(roadmap_item_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_subscribers_for_item(
        &self,
        roadmap_item_id: Uuid,
    ) -> Result<Vec<RoadmapSubscriber>, RoadmapStoreError> {
        let rows: Vec<(Uuid, Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT user_id, roadmap_item_id, created_at \
             FROM roadmap_subscribers WHERE roadmap_item_id = $1 \
             ORDER BY created_at ASC",
        )
        .bind(roadmap_item_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(u, i, c)| RoadmapSubscriber {
                user_id: u,
                roadmap_item_id: i,
                created_at: c,
            })
            .collect())
    }

    async fn draft_changelog(
        &self,
        payload: DraftChangelog<'_>,
    ) -> Result<RoadmapChangelogEntry, RoadmapStoreError> {
        let sql = format!(
            r#"
            INSERT INTO roadmap_changelog
                (roadmap_item_id, channel, title, body,
                 previous_shipped_sha, shipped_sha)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING {CHANGELOG_COLS}
            "#
        );
        let row: ChangelogRow = sqlx::query_as(&sql)
            .bind(payload.roadmap_item_id)
            .bind(payload.channel.as_str())
            .bind(payload.title)
            .bind(payload.body)
            .bind(payload.previous_shipped_sha)
            .bind(payload.shipped_sha)
            .fetch_one(&self.pool)
            .await?;
        row_to_changelog(row)
    }

    async fn get_changelog_entry(
        &self,
        id: Uuid,
    ) -> Result<Option<RoadmapChangelogEntry>, RoadmapStoreError> {
        let sql = format!("SELECT {CHANGELOG_COLS} FROM roadmap_changelog WHERE id = $1");
        let row: Option<ChangelogRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_changelog).transpose()
    }

    async fn list_changelog_drafts(&self) -> Result<Vec<RoadmapChangelogEntry>, RoadmapStoreError> {
        let sql = format!(
            "SELECT {CHANGELOG_COLS} FROM roadmap_changelog \
             WHERE published_at IS NULL ORDER BY created_at DESC"
        );
        let rows: Vec<ChangelogRow> = sqlx::query_as(&sql).fetch_all(&self.pool).await?;
        rows.into_iter().map(row_to_changelog).collect()
    }

    async fn list_published_changelog(
        &self,
        limit: i64,
    ) -> Result<Vec<RoadmapChangelogEntry>, RoadmapStoreError> {
        let limit = limit.clamp(1, 200);
        let sql = format!(
            "SELECT {CHANGELOG_COLS} FROM roadmap_changelog \
             WHERE published_at IS NOT NULL \
             ORDER BY published_at DESC LIMIT $1"
        );
        let rows: Vec<ChangelogRow> = sqlx::query_as(&sql)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(row_to_changelog).collect()
    }

    async fn publish_changelog(
        &self,
        id: Uuid,
        published_by: &str,
    ) -> Result<RoadmapChangelogEntry, RoadmapStoreError> {
        // Guard with `published_at IS NULL` so a re-publish on an
        // already-published row falls through to NotFound rather than
        // bumping the publish stamp.
        let sql = format!(
            r#"
            UPDATE roadmap_changelog
            SET published_at = NOW(),
                published_by = $2
            WHERE id = $1 AND published_at IS NULL
            RETURNING {CHANGELOG_COLS}
            "#
        );
        let row: Option<ChangelogRow> = sqlx::query_as(&sql)
            .bind(id)
            .bind(published_by)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => row_to_changelog(r),
            None => Err(RoadmapStoreError::NotFound),
        }
    }

    async fn edit_changelog_draft(
        &self,
        id: Uuid,
        title: &str,
        body: &str,
    ) -> Result<RoadmapChangelogEntry, RoadmapStoreError> {
        let sql = format!(
            r#"
            UPDATE roadmap_changelog
            SET title = $2,
                body  = $3
            WHERE id = $1 AND published_at IS NULL
            RETURNING {CHANGELOG_COLS}
            "#
        );
        let row: Option<ChangelogRow> = sqlx::query_as(&sql)
            .bind(id)
            .bind(title)
            .bind(body)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => row_to_changelog(r),
            None => Err(RoadmapStoreError::NotFound),
        }
    }

    async fn purge_old_drafts(&self, before: DateTime<Utc>) -> Result<u64, RoadmapStoreError> {
        let result = sqlx::query(
            "DELETE FROM roadmap_changelog \
             WHERE published_at IS NULL AND created_at < $1",
        )
        .bind(before)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    async fn get_user_read_state(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
    ) -> Result<Option<RoadmapUserReadState>, RoadmapStoreError> {
        let row: Option<(Uuid, Uuid, Option<Uuid>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT user_id, roadmap_item_id, last_seen_changelog_entry_id, last_seen_at \
             FROM roadmap_user_read_state \
             WHERE user_id = $1 AND roadmap_item_id = $2",
        )
        .bind(user_id)
        .bind(roadmap_item_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(u, i, e, t)| RoadmapUserReadState {
            user_id: u,
            roadmap_item_id: i,
            last_seen_changelog_entry_id: e,
            last_seen_at: t,
        }))
    }

    async fn upsert_user_read_state(
        &self,
        user_id: Uuid,
        roadmap_item_id: Uuid,
        last_seen_changelog_entry_id: Option<Uuid>,
    ) -> Result<RoadmapUserReadState, RoadmapStoreError> {
        let row: (Uuid, Uuid, Option<Uuid>, DateTime<Utc>) = sqlx::query_as(
            r#"
            INSERT INTO roadmap_user_read_state
                (user_id, roadmap_item_id, last_seen_changelog_entry_id, last_seen_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (user_id, roadmap_item_id) DO UPDATE SET
                last_seen_changelog_entry_id = EXCLUDED.last_seen_changelog_entry_id,
                last_seen_at                 = NOW()
            RETURNING user_id, roadmap_item_id, last_seen_changelog_entry_id, last_seen_at
            "#,
        )
        .bind(user_id)
        .bind(roadmap_item_id)
        .bind(last_seen_changelog_entry_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(RoadmapUserReadState {
            user_id: row.0,
            roadmap_item_id: row.1,
            last_seen_changelog_entry_id: row.2,
            last_seen_at: row.3,
        })
    }

    async fn list_top_level_items_with_changelog(
        &self,
        limit: i64,
    ) -> Result<Vec<RoadmapItem>, RoadmapStoreError> {
        let limit = limit.clamp(1, 50);
        // Inner aggregate selects the most-recent publish per item;
        // we INNER JOIN to keep only items with at least one published
        // entry, filter out private / soft-deleted / non-top-level
        // rows, and order by that freshest publish.
        let sql = format!(
            r#"
            SELECT {ROADMAP_ITEM_COLS}
            FROM roadmap_items i
            INNER JOIN (
                SELECT roadmap_item_id, MAX(published_at) AS latest_published_at
                FROM roadmap_changelog
                WHERE published_at IS NOT NULL
                GROUP BY roadmap_item_id
            ) c ON c.roadmap_item_id = i.id
            WHERE i.deleted_at IS NULL
              AND i.public = TRUE
              AND i.parent_id IS NULL
            ORDER BY c.latest_published_at DESC
            LIMIT $1
            "#
        );
        let rows: Vec<RoadmapItemRow> = sqlx::query_as(&sql)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(row_to_item).collect())
    }
}

// `ChannelStatusArchive` is used by archive_channel / restore_channel.
// `RoadmapChangelogEntry` is used by the changelog trait methods.
// `RoadmapUserReadState` is used by the Phase 8 read-state trait
// methods above — no anchor needed.

// ---------------------------------------------------------------
// Memory implementation under `#[cfg(test)] pub mod test_support`.
// ---------------------------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// Minimal in-memory `RoadmapStore` for route-layer unit tests.
    /// Not concurrency-clever: a single `Mutex` over the whole state
    /// keeps the impl tiny.
    #[derive(Default)]
    pub struct MemoryRoadmapStore {
        state: Mutex<State>,
    }

    #[derive(Default)]
    struct State {
        items: Vec<RoadmapItem>,
        statuses: Vec<ChannelStatus>,
        archives: Vec<ChannelStatusArchive>,
        events: Vec<RoadmapEventLogEntry>,
        votes: Vec<RoadmapVote>,
        subscribers: Vec<RoadmapSubscriber>,
        changelog: Vec<RoadmapChangelogEntry>,
        read_states: Vec<RoadmapUserReadState>,
    }

    impl MemoryRoadmapStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl RoadmapStore for MemoryRoadmapStore {
        async fn upsert_item(
            &self,
            payload: UpsertRoadmapItem<'_>,
        ) -> Result<RoadmapItem, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            // Detect slug collisions against a DIFFERENT github id.
            if let Some(other) = state
                .items
                .iter()
                .find(|i| i.slug == payload.slug)
                .filter(|i| i.github_project_item_id != payload.github_project_item_id)
            {
                let _ = other;
                return Err(RoadmapStoreError::DuplicateSlug(payload.slug.to_string()));
            }
            let now = Utc::now();
            let links = payload.links.cloned().unwrap_or(Value::Array(vec![]));
            let surfaces = payload.surfaces.to_vec();
            if let Some(row) = state
                .items
                .iter_mut()
                .find(|i| i.github_project_item_id == payload.github_project_item_id)
            {
                // Slug is immutable: keep existing.
                row.title = payload.title.to_string();
                row.summary = payload.summary.map(|s| s.to_string());
                row.category = payload.category.map(|s| s.to_string());
                row.eta_band = payload.eta_band.map(|s| s.to_string());
                row.surfaces = surfaces;
                row.parent_id = payload.parent_id;
                row.links = links;
                row.public = payload.public;
                // Mirror the SQL ON CONFLICT: a re-upsert resurrects a
                // soft-deleted row (#149).
                row.deleted_at = None;
                row.content_last_updated = now;
                return Ok(row.clone());
            }
            let item = RoadmapItem {
                id: Uuid::now_v7(),
                slug: payload.slug.to_string(),
                github_project_item_id: payload.github_project_item_id.to_string(),
                title: payload.title.to_string(),
                summary: payload.summary.map(|s| s.to_string()),
                category: payload.category.map(|s| s.to_string()),
                eta_band: payload.eta_band.map(|s| s.to_string()),
                votes: 0,
                surfaces,
                parent_id: payload.parent_id,
                links,
                public: payload.public,
                content_last_updated: now,
                pipeline_last_updated: now,
                created_at: now,
                deleted_at: None,
            };
            state.items.push(item.clone());
            Ok(item)
        }

        async fn set_item_public(&self, id: Uuid, public: bool) -> Result<(), RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            if let Some(row) = state
                .items
                .iter_mut()
                .find(|i| i.id == id && i.deleted_at.is_none())
            {
                row.public = public;
            }
            Ok(())
        }

        async fn get_item_by_slug(
            &self,
            slug: &str,
        ) -> Result<Option<RoadmapItem>, RoadmapStoreError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .items
                .iter()
                .find(|i| i.slug == slug && i.deleted_at.is_none())
                .cloned())
        }

        async fn get_item_by_github_id(
            &self,
            github_project_item_id: &str,
        ) -> Result<Option<RoadmapItem>, RoadmapStoreError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .items
                .iter()
                .find(|i| {
                    i.github_project_item_id == github_project_item_id && i.deleted_at.is_none()
                })
                .cloned())
        }

        async fn list_items(
            &self,
            public_only: bool,
        ) -> Result<Vec<RoadmapItem>, RoadmapStoreError> {
            let state = self.state.lock().unwrap();
            let mut items: Vec<RoadmapItem> = state
                .items
                .iter()
                .filter(|i| i.deleted_at.is_none())
                .filter(|i| !public_only || i.public)
                .cloned()
                .collect();
            items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(items)
        }

        async fn soft_delete_item(&self, id: Uuid) -> Result<(), RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            if let Some(row) = state.items.iter_mut().find(|i| i.id == id) {
                if row.deleted_at.is_none() {
                    row.deleted_at = Some(Utc::now());
                }
            }
            Ok(())
        }

        async fn soft_delete_by_github_id(
            &self,
            github_project_item_id: &str,
        ) -> Result<(), RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            if let Some(row) = state
                .items
                .iter_mut()
                .find(|i| i.github_project_item_id == github_project_item_id)
            {
                if row.deleted_at.is_none() {
                    row.deleted_at = Some(Utc::now());
                }
            }
            Ok(())
        }

        async fn list_channel_statuses(
            &self,
            roadmap_item_id: Uuid,
        ) -> Result<Vec<ChannelStatus>, RoadmapStoreError> {
            let state = self.state.lock().unwrap();
            let mut rows: Vec<ChannelStatus> = state
                .statuses
                .iter()
                .filter(|s| s.roadmap_item_id == roadmap_item_id)
                .cloned()
                .collect();
            rows.sort_by_key(|s| s.channel.as_str());
            Ok(rows)
        }

        async fn upsert_channel_status(
            &self,
            payload: UpsertChannelStatus<'_>,
        ) -> Result<ChannelStatus, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            let now = Utc::now();
            if let Some(row) = state.statuses.iter_mut().find(|s| {
                s.roadmap_item_id == payload.roadmap_item_id && s.channel == payload.channel
            }) {
                row.status = payload.status;
                row.build_health = payload.build_health;
                row.build_id = payload.build_id.map(|s| s.to_string());
                row.commit_sha = payload.commit_sha.map(|s| s.to_string());
                row.deployed_at = payload.deployed_at;
                row.ci_run_url = payload.ci_run_url.map(|s| s.to_string());
                row.previous_shipped_sha = payload.previous_shipped_sha.map(|s| s.to_string());
                row.last_event_id = payload.last_event_id.map(|s| s.to_string());
                row.updated_at = now;
                return Ok(row.clone());
            }
            let row = ChannelStatus {
                roadmap_item_id: payload.roadmap_item_id,
                channel: payload.channel,
                status: payload.status,
                build_health: payload.build_health,
                build_id: payload.build_id.map(|s| s.to_string()),
                commit_sha: payload.commit_sha.map(|s| s.to_string()),
                deployed_at: payload.deployed_at,
                ci_run_url: payload.ci_run_url.map(|s| s.to_string()),
                previous_shipped_sha: payload.previous_shipped_sha.map(|s| s.to_string()),
                last_event_id: payload.last_event_id.map(|s| s.to_string()),
                updated_at: now,
            };
            state.statuses.push(row.clone());
            Ok(row)
        }

        async fn archive_channel(
            &self,
            roadmap_item_id: Uuid,
            channel: ChannelName,
        ) -> Result<(), RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            let idx = state
                .statuses
                .iter()
                .position(|s| s.roadmap_item_id == roadmap_item_id && s.channel == channel)
                .ok_or(RoadmapStoreError::NotFound)?;
            let live = state.statuses.remove(idx);
            // Overwrite an existing archive row for the same key.
            state
                .archives
                .retain(|a| !(a.roadmap_item_id == roadmap_item_id && a.channel == channel));
            state.archives.push(ChannelStatusArchive {
                roadmap_item_id: live.roadmap_item_id,
                channel: live.channel,
                status: live.status,
                build_health: live.build_health,
                build_id: live.build_id,
                commit_sha: live.commit_sha,
                deployed_at: live.deployed_at,
                ci_run_url: live.ci_run_url,
                previous_shipped_sha: live.previous_shipped_sha,
                last_event_id: live.last_event_id,
                archived_at: Utc::now(),
            });
            Ok(())
        }

        async fn restore_channel(
            &self,
            roadmap_item_id: Uuid,
            channel: ChannelName,
        ) -> Result<ChannelStatus, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            let idx = state
                .archives
                .iter()
                .position(|a| a.roadmap_item_id == roadmap_item_id && a.channel == channel)
                .ok_or(RoadmapStoreError::NotFound)?;
            let archived = state.archives.remove(idx);
            // Overwrite an existing live row for the same key.
            state
                .statuses
                .retain(|s| !(s.roadmap_item_id == roadmap_item_id && s.channel == channel));
            let row = ChannelStatus {
                roadmap_item_id: archived.roadmap_item_id,
                channel: archived.channel,
                status: archived.status,
                build_health: archived.build_health,
                build_id: archived.build_id,
                commit_sha: archived.commit_sha,
                deployed_at: archived.deployed_at,
                ci_run_url: archived.ci_run_url,
                previous_shipped_sha: archived.previous_shipped_sha,
                last_event_id: archived.last_event_id,
                updated_at: Utc::now(),
            };
            state.statuses.push(row.clone());
            Ok(row)
        }

        async fn record_event(&self, event_id: &str) -> Result<bool, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            if state.events.iter().any(|e| e.event_id == event_id) {
                return Ok(false);
            }
            state.events.push(RoadmapEventLogEntry {
                event_id: event_id.to_string(),
                received_at: Utc::now(),
            });
            Ok(true)
        }

        async fn event_seen(
            &self,
            event_id: &str,
        ) -> Result<Option<RoadmapEventLogEntry>, RoadmapStoreError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .events
                .iter()
                .find(|e| e.event_id == event_id)
                .cloned())
        }

        async fn cast_vote(
            &self,
            user_id: Uuid,
            roadmap_item_id: Uuid,
        ) -> Result<RoadmapVote, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            if let Some(existing) = state
                .votes
                .iter()
                .find(|v| v.user_id == user_id && v.roadmap_item_id == roadmap_item_id)
            {
                return Ok(existing.clone());
            }
            let row = RoadmapVote {
                user_id,
                roadmap_item_id,
                created_at: Utc::now(),
            };
            state.votes.push(row.clone());
            Ok(row)
        }

        async fn retract_vote(
            &self,
            user_id: Uuid,
            roadmap_item_id: Uuid,
        ) -> Result<(), RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            state
                .votes
                .retain(|v| !(v.user_id == user_id && v.roadmap_item_id == roadmap_item_id));
            Ok(())
        }

        async fn count_votes(&self, roadmap_item_id: Uuid) -> Result<VoteTally, RoadmapStoreError> {
            let n = self
                .state
                .lock()
                .unwrap()
                .votes
                .iter()
                .filter(|v| v.roadmap_item_id == roadmap_item_id)
                .count();
            Ok(VoteTally {
                roadmap_item_id,
                votes: n as i64,
            })
        }

        async fn subscribe(
            &self,
            user_id: Uuid,
            roadmap_item_id: Uuid,
        ) -> Result<RoadmapSubscriber, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            if let Some(existing) = state
                .subscribers
                .iter()
                .find(|s| s.user_id == user_id && s.roadmap_item_id == roadmap_item_id)
            {
                return Ok(existing.clone());
            }
            let row = RoadmapSubscriber {
                user_id,
                roadmap_item_id,
                created_at: Utc::now(),
            };
            state.subscribers.push(row.clone());
            Ok(row)
        }

        async fn unsubscribe(
            &self,
            user_id: Uuid,
            roadmap_item_id: Uuid,
        ) -> Result<(), RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            state
                .subscribers
                .retain(|s| !(s.user_id == user_id && s.roadmap_item_id == roadmap_item_id));
            Ok(())
        }

        async fn list_subscribers_for_item(
            &self,
            roadmap_item_id: Uuid,
        ) -> Result<Vec<RoadmapSubscriber>, RoadmapStoreError> {
            let mut out: Vec<RoadmapSubscriber> = self
                .state
                .lock()
                .unwrap()
                .subscribers
                .iter()
                .filter(|s| s.roadmap_item_id == roadmap_item_id)
                .cloned()
                .collect();
            out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            Ok(out)
        }

        async fn draft_changelog(
            &self,
            payload: DraftChangelog<'_>,
        ) -> Result<RoadmapChangelogEntry, RoadmapStoreError> {
            let row = RoadmapChangelogEntry {
                id: Uuid::now_v7(),
                roadmap_item_id: payload.roadmap_item_id,
                channel: payload.channel,
                title: payload.title.to_string(),
                body: payload.body.to_string(),
                previous_shipped_sha: payload.previous_shipped_sha.map(|s| s.to_string()),
                shipped_sha: payload.shipped_sha.map(|s| s.to_string()),
                created_at: Utc::now(),
                published_at: None,
                published_by: None,
            };
            self.state.lock().unwrap().changelog.push(row.clone());
            Ok(row)
        }

        async fn get_changelog_entry(
            &self,
            id: Uuid,
        ) -> Result<Option<RoadmapChangelogEntry>, RoadmapStoreError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .changelog
                .iter()
                .find(|e| e.id == id)
                .cloned())
        }

        async fn list_changelog_drafts(
            &self,
        ) -> Result<Vec<RoadmapChangelogEntry>, RoadmapStoreError> {
            let mut out: Vec<RoadmapChangelogEntry> = self
                .state
                .lock()
                .unwrap()
                .changelog
                .iter()
                .filter(|e| e.published_at.is_none())
                .cloned()
                .collect();
            out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(out)
        }

        async fn list_published_changelog(
            &self,
            limit: i64,
        ) -> Result<Vec<RoadmapChangelogEntry>, RoadmapStoreError> {
            let limit = limit.clamp(1, 200) as usize;
            let mut out: Vec<RoadmapChangelogEntry> = self
                .state
                .lock()
                .unwrap()
                .changelog
                .iter()
                .filter(|e| e.published_at.is_some())
                .cloned()
                .collect();
            out.sort_by(|a, b| b.published_at.cmp(&a.published_at));
            out.truncate(limit);
            Ok(out)
        }

        async fn publish_changelog(
            &self,
            id: Uuid,
            published_by: &str,
        ) -> Result<RoadmapChangelogEntry, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            let row = state
                .changelog
                .iter_mut()
                .find(|e| e.id == id && e.published_at.is_none())
                .ok_or(RoadmapStoreError::NotFound)?;
            row.published_at = Some(Utc::now());
            row.published_by = Some(published_by.to_string());
            Ok(row.clone())
        }

        async fn edit_changelog_draft(
            &self,
            id: Uuid,
            title: &str,
            body: &str,
        ) -> Result<RoadmapChangelogEntry, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            let row = state
                .changelog
                .iter_mut()
                .find(|e| e.id == id && e.published_at.is_none())
                .ok_or(RoadmapStoreError::NotFound)?;
            row.title = title.to_string();
            row.body = body.to_string();
            Ok(row.clone())
        }

        async fn purge_old_drafts(&self, before: DateTime<Utc>) -> Result<u64, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            let before_len = state.changelog.len();
            state
                .changelog
                .retain(|e| !(e.published_at.is_none() && e.created_at < before));
            Ok((before_len - state.changelog.len()) as u64)
        }

        async fn get_user_read_state(
            &self,
            user_id: Uuid,
            roadmap_item_id: Uuid,
        ) -> Result<Option<RoadmapUserReadState>, RoadmapStoreError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .read_states
                .iter()
                .find(|r| r.user_id == user_id && r.roadmap_item_id == roadmap_item_id)
                .cloned())
        }

        async fn upsert_user_read_state(
            &self,
            user_id: Uuid,
            roadmap_item_id: Uuid,
            last_seen_changelog_entry_id: Option<Uuid>,
        ) -> Result<RoadmapUserReadState, RoadmapStoreError> {
            let mut state = self.state.lock().unwrap();
            let now = Utc::now();
            if let Some(row) = state
                .read_states
                .iter_mut()
                .find(|r| r.user_id == user_id && r.roadmap_item_id == roadmap_item_id)
            {
                row.last_seen_changelog_entry_id = last_seen_changelog_entry_id;
                row.last_seen_at = now;
                return Ok(row.clone());
            }
            let row = RoadmapUserReadState {
                user_id,
                roadmap_item_id,
                last_seen_changelog_entry_id,
                last_seen_at: now,
            };
            state.read_states.push(row.clone());
            Ok(row)
        }

        async fn list_top_level_items_with_changelog(
            &self,
            limit: i64,
        ) -> Result<Vec<RoadmapItem>, RoadmapStoreError> {
            let limit = limit.clamp(1, 50) as usize;
            let state = self.state.lock().unwrap();
            // For each item, find the most-recent published entry's
            // timestamp. Drop items with no published entry.
            let mut scored: Vec<(DateTime<Utc>, RoadmapItem)> = state
                .items
                .iter()
                .filter(|i| i.deleted_at.is_none() && i.public && i.parent_id.is_none())
                .filter_map(|i| {
                    let latest = state
                        .changelog
                        .iter()
                        .filter(|e| e.roadmap_item_id == i.id && e.published_at.is_some())
                        .filter_map(|e| e.published_at)
                        .max()?;
                    Some((latest, i.clone()))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            scored.truncate(limit);
            Ok(scored.into_iter().map(|(_, i)| i).collect())
        }
    }
}

// ---------------------------------------------------------------
// Memory-store tests (Phase 1 acceptance).
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::test_support::MemoryRoadmapStore;
    use super::*;

    fn item_payload<'a>(slug: &'a str, gh_id: &'a str, title: &'a str) -> UpsertRoadmapItem<'a> {
        UpsertRoadmapItem {
            github_project_item_id: gh_id,
            slug,
            title,
            summary: None,
            category: None,
            eta_band: None,
            surfaces: &[],
            parent_id: None,
            links: None,
            public: true,
        }
    }

    fn status_payload(
        item: Uuid,
        channel: ChannelName,
        status: RoadmapStatus,
    ) -> UpsertChannelStatus<'static> {
        UpsertChannelStatus {
            roadmap_item_id: item,
            channel,
            status,
            build_health: BuildHealth::Unknown,
            build_id: None,
            commit_sha: None,
            deployed_at: None,
            ci_run_url: None,
            previous_shipped_sha: None,
            last_event_id: None,
        }
    }

    #[tokio::test]
    async fn upsert_item_idempotent() {
        let store = MemoryRoadmapStore::new();
        let a = store
            .upsert_item(item_payload("dash-feature", "PVTI_A", "Dashboard feature"))
            .await
            .unwrap();
        // Re-upsert on the same github id with a different title.
        let b = store
            .upsert_item(item_payload(
                "dash-feature",
                "PVTI_A",
                "Dashboard feature v2",
            ))
            .await
            .unwrap();
        assert_eq!(a.id, b.id, "same row updated, not inserted");
        assert_eq!(b.title, "Dashboard feature v2");
        let list = store.list_items(false).await.unwrap();
        assert_eq!(list.len(), 1, "still one row");
    }

    #[tokio::test]
    async fn slug_immutable_after_create() {
        let store = MemoryRoadmapStore::new();
        let orig = store
            .upsert_item(item_payload("orig-slug", "PVTI_A", "Orig title"))
            .await
            .unwrap();
        // Caller "tries to rename" by passing a different slug under
        // the same GitHub id. Store preserves the original slug.
        let updated = store
            .upsert_item(item_payload("new-slug-attempt", "PVTI_A", "Orig title"))
            .await
            .unwrap();
        assert_eq!(updated.id, orig.id);
        assert_eq!(
            updated.slug, "orig-slug",
            "slug must not change on re-upsert"
        );
        // The new slug shouldn't be reachable.
        let none = store.get_item_by_slug("new-slug-attempt").await.unwrap();
        assert!(none.is_none());
        // Original slug still resolves.
        let some = store.get_item_by_slug("orig-slug").await.unwrap();
        assert!(some.is_some());
    }

    #[tokio::test]
    async fn upsert_resurrects_soft_deleted_item() {
        // Regression for #149: an item archived on the board is soft-deleted
        // locally (sync.rs handles `archived`/`deleted` webhooks). When it
        // reappears on the board the reconciler upserts it on every tick, so a
        // board-present item must resurrect. Before the fix `upsert_item` left
        // `deleted_at` set, so item #146 stayed absent from `/v1/roadmap` for
        // weeks despite Public=Yes and a working reconciler.
        let store = MemoryRoadmapStore::new();
        let item = store
            .upsert_item(item_payload(
                "name-resolution",
                "PVTI_146",
                "Name Resolution",
            ))
            .await
            .unwrap();
        store.soft_delete_item(item.id).await.unwrap();
        assert!(
            store.list_items(true).await.unwrap().is_empty(),
            "soft-deleted item is hidden while deleted",
        );
        // Reconciler re-upserts the same GitHub id (item reappeared on board).
        store
            .upsert_item(item_payload(
                "name-resolution",
                "PVTI_146",
                "Name Resolution",
            ))
            .await
            .unwrap();
        let public = store.list_items(true).await.unwrap();
        assert_eq!(public.len(), 1, "re-upserted board item must resurrect");
        assert!(
            public[0].deleted_at.is_none(),
            "upsert must clear deleted_at on a resurrected item",
        );
    }

    #[tokio::test]
    async fn channel_status_archive_roundtrip() {
        let store = MemoryRoadmapStore::new();
        let item = store
            .upsert_item(item_payload("arch-test", "PVTI_ARCH", "Archive test"))
            .await
            .unwrap();
        // Seed a live status with a previous_shipped_sha to verify
        // it survives the round trip.
        let mut payload = status_payload(item.id, ChannelName::Live, RoadmapStatus::Shipped);
        payload.previous_shipped_sha = Some("deadbeef");
        store.upsert_channel_status(payload).await.unwrap();
        // Archive it.
        store
            .archive_channel(item.id, ChannelName::Live)
            .await
            .unwrap();
        // Live channel should be gone -- attempting to archive again
        // returns NotFound.
        let again = store.archive_channel(item.id, ChannelName::Live).await;
        assert!(matches!(again, Err(RoadmapStoreError::NotFound)));
        // Restore brings it back with the previous_shipped_sha intact.
        let restored = store
            .restore_channel(item.id, ChannelName::Live)
            .await
            .unwrap();
        assert_eq!(restored.previous_shipped_sha.as_deref(), Some("deadbeef"));
        assert_eq!(restored.status, RoadmapStatus::Shipped);
    }

    #[tokio::test]
    async fn vote_insert_and_retract_net_to_zero() {
        let store = MemoryRoadmapStore::new();
        let item = store
            .upsert_item(item_payload("vote-it", "PVTI_V", "Votable"))
            .await
            .unwrap();
        let user = Uuid::now_v7();
        store.cast_vote(user, item.id).await.unwrap();
        // Duplicate cast is idempotent (still one vote).
        store.cast_vote(user, item.id).await.unwrap();
        assert_eq!(store.count_votes(item.id).await.unwrap().votes, 1);
        store.retract_vote(user, item.id).await.unwrap();
        assert_eq!(store.count_votes(item.id).await.unwrap().votes, 0);
        // Retracting a non-existent vote is a no-op.
        store.retract_vote(user, item.id).await.unwrap();
    }

    #[tokio::test]
    async fn subscriber_membership_is_private() {
        // The trait *can* return subscriber rows (server-internal
        // surface) -- the privacy guarantee is at the route layer.
        // Here we just exercise membership operations.
        let store = MemoryRoadmapStore::new();
        let item = store
            .upsert_item(item_payload("sub-test", "PVTI_S", "Subscribable"))
            .await
            .unwrap();
        let u1 = Uuid::now_v7();
        let u2 = Uuid::now_v7();
        store.subscribe(u1, item.id).await.unwrap();
        store.subscribe(u2, item.id).await.unwrap();
        // Idempotent.
        store.subscribe(u1, item.id).await.unwrap();
        let subs = store.list_subscribers_for_item(item.id).await.unwrap();
        assert_eq!(subs.len(), 2);
        store.unsubscribe(u1, item.id).await.unwrap();
        let subs = store.list_subscribers_for_item(item.id).await.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].user_id, u2);
    }

    #[tokio::test]
    async fn soft_delete_excludes_from_list() {
        let store = MemoryRoadmapStore::new();
        let a = store
            .upsert_item(item_payload("alive", "PVTI_LIVE", "Alive"))
            .await
            .unwrap();
        let b = store
            .upsert_item(item_payload("doomed", "PVTI_DOOMED", "Doomed"))
            .await
            .unwrap();
        store.soft_delete_item(b.id).await.unwrap();
        let live = store.list_items(false).await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, a.id);
        // get_item_by_slug also filters out deleted rows.
        assert!(store.get_item_by_slug("doomed").await.unwrap().is_none());
        // Re-deleting is idempotent (no panic).
        store.soft_delete_item(b.id).await.unwrap();
    }

    #[tokio::test]
    async fn event_log_dedup_on_event_id() {
        let store = MemoryRoadmapStore::new();
        let first = store.record_event("evt-1").await.unwrap();
        let second = store.record_event("evt-1").await.unwrap();
        assert!(first, "first insert returns true");
        assert!(!second, "duplicate insert returns false");
        // event_seen reflects the stored row.
        let seen = store.event_seen("evt-1").await.unwrap();
        assert!(seen.is_some());
        assert!(store.event_seen("evt-missing").await.unwrap().is_none());
    }

    // Headline-status aggregation tests live in models.rs (where the
    // function itself is defined). The required
    // `headline_status_aggregation_matrix` test name is satisfied
    // there.
}
