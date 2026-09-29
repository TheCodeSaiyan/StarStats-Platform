//! Chat reports (social phase 6). See migration 0080 for what is kept and
//! why; `chat_report_routes.rs` for who can file and resolve them.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

/// Reports a player can file in a day, as for LFG and share reports.
pub const REPORTS_PER_DAY: i64 = 5;
/// Messages one report can reveal, and how long each may be (the web
/// composer's own limit).
pub const MAX_REVEALED: usize = 20;
pub const MAX_MESSAGE_CHARS: usize = 2000;
pub const DETAILS_MAX: usize = 500;
pub const RESOLUTION_NOTE_MAX: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum ChatReportError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("bad stored value: {0}")]
    Domain(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatReportReason {
    Harassment,
    Spam,
    Scam,
    IllegalContent,
    Other,
}

impl ChatReportReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Harassment => "harassment",
            Self::Spam => "spam",
            Self::Scam => "scam",
            Self::IllegalContent => "illegal_content",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Harassment,
            Self::Spam,
            Self::Scam,
            Self::IllegalContent,
            Self::Other,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatReportStatus {
    Open,
    Dismissed,
    ChatRestricted,
    UserSuspended,
}

impl ChatReportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Dismissed => "dismissed",
            Self::ChatRestricted => "chat_restricted",
            Self::UserSuspended => "user_suspended",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Open,
            Self::Dismissed,
            Self::ChatRestricted,
            Self::UserSuspended,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }
}

/// One message as the reporter revealed it. Not proof: Matrix has no
/// message franking, so the sender is the reporter's word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RevealedMessage {
    pub event_id: String,
    pub sender: String,
    pub sent_at: DateTime<Utc>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatReport {
    pub id: Uuid,
    pub reporter_handle: String,
    pub reported_handle: String,
    pub room_id: String,
    pub reason: ChatReportReason,
    pub details: Option<String>,
    pub messages: Vec<RevealedMessage>,
    pub status: ChatReportStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<String>,
    pub resolution_note: Option<String>,
}

pub struct NewChatReport<'a> {
    pub reporter: &'a str,
    pub reported: &'a str,
    pub room_id: &'a str,
    pub reason: ChatReportReason,
    pub details: Option<&'a str>,
    pub messages: &'a [RevealedMessage],
}

#[async_trait]
pub trait ChatReportStore: Send + Sync + 'static {
    async fn create(&self, r: NewChatReport<'_>) -> Result<ChatReport, ChatReportError>;
    async fn count_since(
        &self,
        reporter: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, ChatReportError>;
    async fn list(
        &self,
        status: Option<ChatReportStatus>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChatReport>, ChatReportError>;
    async fn get(&self, id: Uuid) -> Result<Option<ChatReport>, ChatReportError>;
    async fn resolve(
        &self,
        id: Uuid,
        by: &str,
        status: ChatReportStatus,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<ChatReport, ChatReportError>;
}

type Row = (
    Uuid,
    String,
    String,
    String,
    String,
    Option<String>,
    serde_json::Value,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
);

const COLUMNS: &str = "id, reporter_handle, reported_handle, room_id, reason, details, messages,
    status, created_at, resolved_at, resolved_by, resolution_note";

fn from_row(r: Row) -> Result<ChatReport, ChatReportError> {
    Ok(ChatReport {
        id: r.0,
        reporter_handle: r.1,
        reported_handle: r.2,
        room_id: r.3,
        reason: ChatReportReason::parse(&r.4)
            .ok_or_else(|| ChatReportError::Domain(format!("reason={}", r.4)))?,
        details: r.5,
        messages: serde_json::from_value(r.6)
            .map_err(|e| ChatReportError::Domain(format!("messages: {e}")))?,
        status: ChatReportStatus::parse(&r.7)
            .ok_or_else(|| ChatReportError::Domain(format!("status={}", r.7)))?,
        created_at: r.8,
        resolved_at: r.9,
        resolved_by: r.10,
        resolution_note: r.11,
    })
}

pub struct PostgresChatReportStore {
    pool: PgPool,
}

impl PostgresChatReportStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChatReportStore for PostgresChatReportStore {
    async fn create(&self, r: NewChatReport<'_>) -> Result<ChatReport, ChatReportError> {
        let messages =
            serde_json::to_value(r.messages).map_err(|e| ChatReportError::Domain(e.to_string()))?;
        let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO chat_reports
                 (reporter_handle, reported_handle, room_id, reason, details, messages)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING {COLUMNS}"
        )))
        .bind(r.reporter)
        .bind(r.reported)
        .bind(r.room_id)
        .bind(r.reason.as_str())
        .bind(r.details)
        .bind(messages)
        .fetch_one(&self.pool)
        .await?;
        from_row(row)
    }

    async fn count_since(
        &self,
        reporter: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, ChatReportError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM chat_reports
             WHERE lower(reporter_handle) = lower($1) AND created_at >= $2",
        )
        .bind(reporter)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    async fn list(
        &self,
        status: Option<ChatReportStatus>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChatReport>, ChatReportError> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM chat_reports
             WHERE ($1::text IS NULL OR status = $1)
             ORDER BY created_at DESC
             LIMIT $2 OFFSET $3"
        )))
        .bind(status.map(|s| s.as_str()))
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(from_row).collect()
    }

    async fn get(&self, id: Uuid) -> Result<Option<ChatReport>, ChatReportError> {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM chat_reports WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(from_row).transpose()
    }

    async fn resolve(
        &self,
        id: Uuid,
        by: &str,
        status: ChatReportStatus,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<ChatReport, ChatReportError> {
        let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_reports
             SET status = $2, resolved_by = $3, resolution_note = $4, resolved_at = $5
             WHERE id = $1
             RETURNING {COLUMNS}"
        )))
        .bind(id)
        .bind(status.as_str())
        .bind(by)
        .bind(note)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        from_row(row)
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryChatReportStore {
        rows: Mutex<Vec<ChatReport>>,
    }

    impl MemoryChatReportStore {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn all(&self) -> Vec<ChatReport> {
            self.rows.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChatReportStore for MemoryChatReportStore {
        async fn create(&self, r: NewChatReport<'_>) -> Result<ChatReport, ChatReportError> {
            let report = ChatReport {
                id: Uuid::new_v4(),
                reporter_handle: r.reporter.into(),
                reported_handle: r.reported.into(),
                room_id: r.room_id.into(),
                reason: r.reason,
                details: r.details.map(str::to_string),
                messages: r.messages.to_vec(),
                status: ChatReportStatus::Open,
                created_at: Utc::now(),
                resolved_at: None,
                resolved_by: None,
                resolution_note: None,
            };
            self.rows.lock().unwrap().push(report.clone());
            Ok(report)
        }

        async fn count_since(
            &self,
            reporter: &str,
            since: DateTime<Utc>,
        ) -> Result<i64, ChatReportError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|r| {
                    r.reporter_handle.eq_ignore_ascii_case(reporter) && r.created_at >= since
                })
                .count() as i64)
        }

        async fn list(
            &self,
            status: Option<ChatReportStatus>,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<ChatReport>, ChatReportError> {
            let mut v: Vec<ChatReport> = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|r| status.is_none_or(|s| r.status == s))
                .cloned()
                .collect();
            v.sort_by_key(|r| std::cmp::Reverse(r.created_at));
            Ok(v.into_iter()
                .skip(offset.max(0) as usize)
                .take(limit.max(0) as usize)
                .collect())
        }

        async fn get(&self, id: Uuid) -> Result<Option<ChatReport>, ChatReportError> {
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
            by: &str,
            status: ChatReportStatus,
            note: Option<&str>,
            now: DateTime<Utc>,
        ) -> Result<ChatReport, ChatReportError> {
            let mut rows = self.rows.lock().unwrap();
            let r = rows
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or_else(|| ChatReportError::Domain("no such report".into()))?;
            r.status = status;
            r.resolved_by = Some(by.into());
            r.resolution_note = note.map(str::to_string);
            r.resolved_at = Some(now);
            Ok(r.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabularies_round_trip() {
        for r in ["harassment", "spam", "scam", "illegal_content", "other"] {
            assert_eq!(ChatReportReason::parse(r).unwrap().as_str(), r);
        }
        for s in ["open", "dismissed", "chat_restricted", "user_suspended"] {
            assert_eq!(ChatReportStatus::parse(s).unwrap().as_str(), s);
        }
        assert!(ChatReportReason::parse("rude").is_none());
    }

    /// The store's SQL against the real schema. Skipped without
    /// STARSTATS_TEST_DATABASE_URL.
    #[tokio::test]
    async fn postgres_chat_report_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres chat report test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");
        let s = PostgresChatReportStore::new(pool.clone());
        let tag = &Uuid::new_v4().simple().to_string()[..8];
        let reporter = format!("Reporter{tag}");
        let msgs = vec![RevealedMessage {
            event_id: "$e1".into(),
            sender: "@mallory:starstats.app".into(),
            sent_at: Utc::now(),
            text: "rude words".into(),
        }];
        let r = s
            .create(NewChatReport {
                reporter: &reporter,
                reported: "Mallory",
                room_id: "!r:starstats.app",
                reason: ChatReportReason::Harassment,
                details: Some("kept going"),
                messages: &msgs,
            })
            .await
            .unwrap();
        assert_eq!(r.messages, msgs);
        assert_eq!(r.status, ChatReportStatus::Open);
        assert_eq!(
            s.count_since(
                &reporter.to_uppercase(),
                Utc::now() - chrono::Duration::hours(1)
            )
            .await
            .unwrap(),
            1
        );
        assert!(s
            .list(Some(ChatReportStatus::Open), 200, 0)
            .await
            .unwrap()
            .iter()
            .any(|x| x.id == r.id));
        let done = s
            .resolve(
                r.id,
                "mod",
                ChatReportStatus::ChatRestricted,
                Some("warned"),
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(done.status, ChatReportStatus::ChatRestricted);
        assert_eq!(
            s.get(r.id).await.unwrap().unwrap().resolved_by.as_deref(),
            Some("mod")
        );
        sqlx::query("DELETE FROM chat_reports WHERE id = $1")
            .bind(r.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
