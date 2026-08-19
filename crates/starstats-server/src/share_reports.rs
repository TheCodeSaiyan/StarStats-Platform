//! Share-reports moderation queue (audit v2 §05).
//!
//! Reporters file a complaint against a specific
//! (owner_handle, recipient_handle) share. Reports start as `open`,
//! get triaged via `/v1/admin/sharing/reports`, and resolve to one of
//! `dismissed | share_revoked | user_suspended`.
//!
//! Backed by table `share_reports` (migration 0027). Reason +
//! status vocabularies are closed at the application layer; the
//! DB stores them as plain TEXT so adding a variant doesn't require
//! a migration.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

/// Maximum length of the free-text `details` field on a new report.
/// Long enough for a paragraph of context, short enough to keep
/// payloads tight and discourage essay-length grievances.
pub const DETAILS_MAX_LEN: usize = 500;
/// Cap on the moderator's `resolution_note`.
pub const RESOLUTION_NOTE_MAX_LEN: usize = 500;

/// Per-reporter rate-limit window. The create handler counts the
/// reporter's own rows created since `now - WINDOW` and rejects
/// once the count reaches `RATE_LIMIT_PER_WINDOW`.
pub const RATE_LIMIT_WINDOW_HOURS: i64 = 24;
pub const RATE_LIMIT_PER_WINDOW: i64 = 5;

pub fn rate_limit_window() -> Duration {
    Duration::hours(RATE_LIMIT_WINDOW_HOURS)
}

/// Closed status vocabulary. Stored as TEXT at the DB layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShareReportStatus {
    Open,
    Dismissed,
    ShareRevoked,
    UserSuspended,
}

impl ShareReportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Dismissed => "dismissed",
            Self::ShareRevoked => "share_revoked",
            Self::UserSuspended => "user_suspended",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "open" => Self::Open,
            "dismissed" => Self::Dismissed,
            "share_revoked" => Self::ShareRevoked,
            "user_suspended" => Self::UserSuspended,
            _ => return None,
        })
    }

    /// Variants a moderator may transition an `open` report into.
    /// Excludes `open` (the starting state).
    pub fn is_resolution(self) -> bool {
        matches!(
            self,
            Self::Dismissed | Self::ShareRevoked | Self::UserSuspended
        )
    }
}

/// Closed reason vocabulary. Same TEXT storage as status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShareReportReason {
    Abuse,
    Spam,
    DataMisuse,
    Other,
}

impl ShareReportReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Abuse => "abuse",
            Self::Spam => "spam",
            Self::DataMisuse => "data_misuse",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "abuse" => Self::Abuse,
            "spam" => Self::Spam,
            "data_misuse" => Self::DataMisuse,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// One row of the `share_reports` table.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ShareReport {
    pub id: Uuid,
    pub reporter_handle: String,
    pub owner_handle: String,
    pub recipient_handle: String,
    pub reason: ShareReportReason,
    pub details: Option<String>,
    pub status: ShareReportStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<String>,
    pub resolution_note: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ShareReportError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("report not found")]
    NotFound,
    #[error("report already resolved")]
    AlreadyResolved,
    #[error("stored value out of domain: {0}")]
    Domain(String),
}

#[async_trait]
pub trait ShareReportStore: Send + Sync + 'static {
    /// Create a new `open` report. The DB stamps `created_at`; the
    /// caller never overrides it.
    async fn create(
        &self,
        reporter_handle: &str,
        owner_handle: &str,
        recipient_handle: &str,
        reason: ShareReportReason,
        details: Option<&str>,
    ) -> Result<ShareReport, ShareReportError>;

    /// List reports filtered by status, most recent first. `None`
    /// returns every row (capped by `limit`).
    async fn list(
        &self,
        status: Option<ShareReportStatus>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ShareReport>, ShareReportError>;

    /// Single-row lookup. Kept on the trait for future "view this
    /// report" admin pages and exercised by the in-memory test
    /// store; no public route consumes it yet.
    #[allow(dead_code)]
    async fn get(&self, id: Uuid) -> Result<Option<ShareReport>, ShareReportError>;

    /// Transition an `open` report to a resolution status. Returns
    /// `AlreadyResolved` if the row is no longer `open`.
    async fn resolve(
        &self,
        id: Uuid,
        moderator_handle: &str,
        outcome: ShareReportStatus,
        note: Option<&str>,
    ) -> Result<ShareReport, ShareReportError>;

    /// Count reports filed by `reporter_handle` since `since`. Used
    /// by the create handler for rate-limit decisions. Case-
    /// insensitive on handle to match the indexed lookup pattern.
    async fn count_recent_by_reporter(
        &self,
        reporter_handle: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, ShareReportError>;
}

pub struct PostgresShareReportStore {
    pool: PgPool,
}

impl PostgresShareReportStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[allow(clippy::type_complexity)]
type ShareReportRow = (
    Uuid,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
);

fn row_to_report(row: ShareReportRow) -> Result<ShareReport, ShareReportError> {
    let reason = ShareReportReason::parse(&row.4)
        .ok_or_else(|| ShareReportError::Domain(format!("reason={}", row.4)))?;
    let status = ShareReportStatus::parse(&row.6)
        .ok_or_else(|| ShareReportError::Domain(format!("status={}", row.6)))?;
    Ok(ShareReport {
        id: row.0,
        reporter_handle: row.1,
        owner_handle: row.2,
        recipient_handle: row.3,
        reason,
        details: row.5,
        status,
        created_at: row.7,
        resolved_at: row.8,
        resolved_by: row.9,
        resolution_note: row.10,
    })
}

#[async_trait]
impl ShareReportStore for PostgresShareReportStore {
    async fn create(
        &self,
        reporter_handle: &str,
        owner_handle: &str,
        recipient_handle: &str,
        reason: ShareReportReason,
        details: Option<&str>,
    ) -> Result<ShareReport, ShareReportError> {
        let row: ShareReportRow = sqlx::query_as(
            r#"
            INSERT INTO share_reports
                (reporter_handle, owner_handle, recipient_handle, reason, details)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, reporter_handle, owner_handle, recipient_handle,
                      reason, details, status, created_at,
                      resolved_at, resolved_by, resolution_note
            "#,
        )
        .bind(reporter_handle)
        .bind(owner_handle)
        .bind(recipient_handle)
        .bind(reason.as_str())
        .bind(details)
        .fetch_one(&self.pool)
        .await?;
        row_to_report(row)
    }

    async fn list(
        &self,
        status: Option<ShareReportStatus>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ShareReport>, ShareReportError> {
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let rows: Vec<ShareReportRow> = if let Some(s) = status {
            sqlx::query_as(
                r#"
                SELECT id, reporter_handle, owner_handle, recipient_handle,
                       reason, details, status, created_at,
                       resolved_at, resolved_by, resolution_note
                FROM share_reports
                WHERE status = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(s.as_str())
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                r#"
                SELECT id, reporter_handle, owner_handle, recipient_handle,
                       reason, details, status, created_at,
                       resolved_at, resolved_by, resolution_note
                FROM share_reports
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(row_to_report).collect()
    }

    async fn get(&self, id: Uuid) -> Result<Option<ShareReport>, ShareReportError> {
        let row: Option<ShareReportRow> = sqlx::query_as(
            r#"
            SELECT id, reporter_handle, owner_handle, recipient_handle,
                   reason, details, status, created_at,
                   resolved_at, resolved_by, resolution_note
            FROM share_reports
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_report).transpose()
    }

    async fn resolve(
        &self,
        id: Uuid,
        moderator_handle: &str,
        outcome: ShareReportStatus,
        note: Option<&str>,
    ) -> Result<ShareReport, ShareReportError> {
        // Guarded with `status = 'open'` so a stale double-resolve
        // returns 0 rows, which we translate to `AlreadyResolved`.
        let row: Option<ShareReportRow> = sqlx::query_as(
            r#"
            UPDATE share_reports
            SET status          = $2,
                resolved_at     = NOW(),
                resolved_by     = $3,
                resolution_note = $4
            WHERE id = $1 AND status = 'open'
            RETURNING id, reporter_handle, owner_handle, recipient_handle,
                      reason, details, status, created_at,
                      resolved_at, resolved_by, resolution_note
            "#,
        )
        .bind(id)
        .bind(outcome.as_str())
        .bind(moderator_handle)
        .bind(note)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(r) => row_to_report(r),
            None => {
                // Distinguish "missing" from "already resolved" by
                // checking row presence.
                let exists: Option<(Uuid,)> =
                    sqlx::query_as("SELECT id FROM share_reports WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&self.pool)
                        .await?;
                if exists.is_some() {
                    Err(ShareReportError::AlreadyResolved)
                } else {
                    Err(ShareReportError::NotFound)
                }
            }
        }
    }

    async fn count_recent_by_reporter(
        &self,
        reporter_handle: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, ShareReportError> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint
            FROM share_reports
            WHERE lower(reporter_handle) = lower($1)
              AND created_at >= $2
            "#,
        )
        .bind(reporter_handle)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryShareReportStore {
        rows: Mutex<Vec<ShareReport>>,
    }

    impl MemoryShareReportStore {
        pub fn new() -> Self {
            Self::default()
        }

        #[allow(dead_code)]
        pub fn snapshot(&self) -> Vec<ShareReport> {
            self.rows.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ShareReportStore for MemoryShareReportStore {
        async fn create(
            &self,
            reporter_handle: &str,
            owner_handle: &str,
            recipient_handle: &str,
            reason: ShareReportReason,
            details: Option<&str>,
        ) -> Result<ShareReport, ShareReportError> {
            let report = ShareReport {
                id: Uuid::now_v7(),
                reporter_handle: reporter_handle.into(),
                owner_handle: owner_handle.into(),
                recipient_handle: recipient_handle.into(),
                reason,
                details: details.map(|s| s.to_string()),
                status: ShareReportStatus::Open,
                created_at: Utc::now(),
                resolved_at: None,
                resolved_by: None,
                resolution_note: None,
            };
            self.rows.lock().unwrap().push(report.clone());
            Ok(report)
        }

        async fn list(
            &self,
            status: Option<ShareReportStatus>,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<ShareReport>, ShareReportError> {
            let limit = limit.clamp(1, 200) as usize;
            let offset = offset.max(0) as usize;
            let snap = self.rows.lock().unwrap();
            let mut filtered: Vec<ShareReport> = snap
                .iter()
                .filter(|r| status.map(|s| r.status == s).unwrap_or(true))
                .cloned()
                .collect();
            // Most-recent first.
            filtered.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(filtered.into_iter().skip(offset).take(limit).collect())
        }

        async fn get(&self, id: Uuid) -> Result<Option<ShareReport>, ShareReportError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.id == id)
                .cloned())
        }

        async fn resolve(
            &self,
            id: Uuid,
            moderator_handle: &str,
            outcome: ShareReportStatus,
            note: Option<&str>,
        ) -> Result<ShareReport, ShareReportError> {
            let mut rows = self.rows.lock().unwrap();
            let row = rows
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or(ShareReportError::NotFound)?;
            if row.status != ShareReportStatus::Open {
                return Err(ShareReportError::AlreadyResolved);
            }
            row.status = outcome;
            row.resolved_at = Some(Utc::now());
            row.resolved_by = Some(moderator_handle.to_string());
            row.resolution_note = note.map(|s| s.to_string());
            Ok(row.clone())
        }

        async fn count_recent_by_reporter(
            &self,
            reporter_handle: &str,
            since: DateTime<Utc>,
        ) -> Result<i64, ShareReportError> {
            let needle = reporter_handle.to_ascii_lowercase();
            let n = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|r| {
                    r.reporter_handle.to_ascii_lowercase() == needle && r.created_at >= since
                })
                .count();
            Ok(n as i64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::MemoryShareReportStore;

    #[test]
    fn status_round_trips_through_str() {
        for s in [
            ShareReportStatus::Open,
            ShareReportStatus::Dismissed,
            ShareReportStatus::ShareRevoked,
            ShareReportStatus::UserSuspended,
        ] {
            assert_eq!(ShareReportStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(ShareReportStatus::parse("bogus"), None);
    }

    #[test]
    fn reason_round_trips_through_str() {
        for r in [
            ShareReportReason::Abuse,
            ShareReportReason::Spam,
            ShareReportReason::DataMisuse,
            ShareReportReason::Other,
        ] {
            assert_eq!(ShareReportReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(ShareReportReason::parse("bogus"), None);
    }

    #[test]
    fn open_is_not_resolution_but_others_are() {
        assert!(!ShareReportStatus::Open.is_resolution());
        assert!(ShareReportStatus::Dismissed.is_resolution());
        assert!(ShareReportStatus::ShareRevoked.is_resolution());
        assert!(ShareReportStatus::UserSuspended.is_resolution());
    }

    #[tokio::test]
    async fn memory_store_create_and_get_round_trips() {
        let store = MemoryShareReportStore::new();
        let r = store
            .create(
                "reporter",
                "owner",
                "recipient",
                ShareReportReason::Abuse,
                Some("the details"),
            )
            .await
            .unwrap();
        assert_eq!(r.status, ShareReportStatus::Open);
        assert_eq!(r.reason, ShareReportReason::Abuse);
        assert_eq!(r.details.as_deref(), Some("the details"));
        let got = store.get(r.id).await.unwrap().unwrap();
        assert_eq!(got.id, r.id);
    }

    #[tokio::test]
    async fn memory_store_list_filters_by_status_and_orders_recent_first() {
        let store = MemoryShareReportStore::new();
        let _a = store
            .create("rp", "o1", "r1", ShareReportReason::Spam, None)
            .await
            .unwrap();
        // 2ms gap so the order is deterministic on fast clocks.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let b = store
            .create("rp", "o2", "r2", ShareReportReason::Spam, None)
            .await
            .unwrap();
        let open = store
            .list(Some(ShareReportStatus::Open), 50, 0)
            .await
            .unwrap();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].id, b.id, "most recent first");

        store
            .resolve(b.id, "mod", ShareReportStatus::Dismissed, None)
            .await
            .unwrap();
        let open = store
            .list(Some(ShareReportStatus::Open), 50, 0)
            .await
            .unwrap();
        assert_eq!(open.len(), 1);
        let all = store.list(None, 50, 0).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn memory_store_resolve_rejects_double_resolution() {
        let store = MemoryShareReportStore::new();
        let r = store
            .create("rp", "o", "r", ShareReportReason::Other, None)
            .await
            .unwrap();
        store
            .resolve(r.id, "mod", ShareReportStatus::Dismissed, Some("nope"))
            .await
            .unwrap();
        let again = store
            .resolve(r.id, "mod", ShareReportStatus::ShareRevoked, None)
            .await;
        assert!(matches!(again, Err(ShareReportError::AlreadyResolved)));
    }

    #[tokio::test]
    async fn memory_store_resolve_unknown_id_is_not_found() {
        let store = MemoryShareReportStore::new();
        let err = store
            .resolve(Uuid::now_v7(), "mod", ShareReportStatus::Dismissed, None)
            .await;
        assert!(matches!(err, Err(ShareReportError::NotFound)));
    }

    #[tokio::test]
    async fn memory_store_count_recent_by_reporter_is_case_insensitive() {
        let store = MemoryShareReportStore::new();
        let before = Utc::now() - Duration::hours(1);
        for _ in 0..3 {
            store
                .create("Alice", "o", "r", ShareReportReason::Spam, None)
                .await
                .unwrap();
        }
        store
            .create("bob", "o", "r", ShareReportReason::Spam, None)
            .await
            .unwrap();
        let n = store
            .count_recent_by_reporter("alice", before)
            .await
            .unwrap();
        assert_eq!(n, 3);
    }
}
