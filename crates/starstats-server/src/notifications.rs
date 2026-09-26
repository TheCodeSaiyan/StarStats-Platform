//! In-app notifications inbox (social phase 1).
//!
//! One row per thing a user should hear about: a friend request, an
//! accepted request. Clients poll `GET /v1/me/notifications?since=` in
//! phase 1; the realtime gateway (phase 3) pushes the same rows.
//!
//! `kind` is a closed vocabulary stored as TEXT. `payload` carries the
//! few fields a client needs to act from the notification alone (the
//! request id, whether the actor's handle is RSI-verified) so a toast
//! can offer Accept or Copy handle without a second round trip.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

/// Hard cap on one page. Clients poll with `since`, so a page this big
/// only happens after a long absence.
pub const PAGE_LIMIT_MAX: i64 = 200;

/// Notifications older than this are deleted. They are prompts to act,
/// not a record; keeping them longer only keeps personal data longer.
pub const RETENTION_DAYS: i64 = 90;

pub fn retention_window() -> Duration {
    Duration::days(RETENTION_DAYS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    FriendRequest,
    FriendAccepted,
}

impl NotificationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FriendRequest => "friend_request",
            Self::FriendAccepted => "friend_accepted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "friend_request" => Self::FriendRequest,
            "friend_accepted" => Self::FriendAccepted,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Notification {
    pub id: Uuid,
    pub kind: NotificationKind,
    pub actor_handle: Option<String>,
    #[schema(value_type = Object)]
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("stored value out of domain: {0}")]
    Domain(String),
}

#[async_trait]
pub trait NotificationStore: Send + Sync + 'static {
    async fn create(
        &self,
        recipient: &str,
        kind: NotificationKind,
        actor: Option<&str>,
        payload: serde_json::Value,
    ) -> Result<Notification, NotificationError>;

    /// Newest first. With `since`, only rows created strictly after it.
    async fn list(
        &self,
        recipient: &str,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Notification>, NotificationError>;

    async fn unread_count(&self, recipient: &str) -> Result<i64, NotificationError>;

    /// Marks only the recipient's own rows; ids belonging to someone
    /// else are ignored rather than rejected, so the call cannot be
    /// used to probe which ids exist.
    async fn mark_read(&self, recipient: &str, ids: &[Uuid]) -> Result<u64, NotificationError>;
    async fn mark_all_read(&self, recipient: &str) -> Result<u64, NotificationError>;

    /// Remove everything `actor` caused in `recipient`'s inbox. Used on
    /// block, so a blocked user's request does not linger as a prompt.
    async fn delete_from_actor(
        &self,
        recipient: &str,
        actor: &str,
    ) -> Result<u64, NotificationError>;

    /// Retention sweep: delete rows created before `before`.
    async fn purge_older_than(&self, before: DateTime<Utc>) -> Result<u64, NotificationError>;
}

/// Daily retention sweep. A failed pass is logged and retried at the
/// next tick; notifications are low-stakes, so there is no backoff.
pub fn spawn_purge_loop(store: std::sync::Arc<dyn NotificationStore>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        loop {
            tick.tick().await;
            let cutoff = Utc::now() - retention_window();
            match store.purge_older_than(cutoff).await {
                Ok(n) => tracing::info!(purged = n, "notifications retention sweep"),
                Err(e) => tracing::warn!(error = %e, "notifications retention sweep failed"),
            }
        }
    });
}

pub struct PostgresNotificationStore {
    pool: PgPool,
}

impl PostgresNotificationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type NotificationRow = (
    Uuid,
    String,
    Option<String>,
    serde_json::Value,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

fn row_to_notification(row: NotificationRow) -> Result<Notification, NotificationError> {
    let kind = NotificationKind::parse(&row.1)
        .ok_or_else(|| NotificationError::Domain(format!("kind={}", row.1)))?;
    Ok(Notification {
        id: row.0,
        kind,
        actor_handle: row.2,
        payload: row.3,
        created_at: row.4,
        read_at: row.5,
    })
}

#[async_trait]
impl NotificationStore for PostgresNotificationStore {
    async fn create(
        &self,
        recipient: &str,
        kind: NotificationKind,
        actor: Option<&str>,
        payload: serde_json::Value,
    ) -> Result<Notification, NotificationError> {
        let row: NotificationRow = sqlx::query_as(
            r#"
            INSERT INTO notifications (recipient_handle, kind, actor_handle, payload)
            VALUES ($1, $2, $3, $4)
            RETURNING id, kind, actor_handle, payload, created_at, read_at
            "#,
        )
        .bind(recipient)
        .bind(kind.as_str())
        .bind(actor)
        .bind(payload)
        .fetch_one(&self.pool)
        .await?;
        row_to_notification(row)
    }

    async fn list(
        &self,
        recipient: &str,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Notification>, NotificationError> {
        let limit = limit.clamp(1, PAGE_LIMIT_MAX);
        // `$2 IS NULL OR created_at > $2` keeps one prepared statement
        // for both the first load and the `since` poll.
        let rows: Vec<NotificationRow> = sqlx::query_as(
            r#"
            SELECT id, kind, actor_handle, payload, created_at, read_at
            FROM notifications
            WHERE lower(recipient_handle) = lower($1)
              AND ($2::timestamptz IS NULL OR created_at > $2)
            ORDER BY created_at DESC
            LIMIT $3
            "#,
        )
        .bind(recipient)
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_notification).collect()
    }

    async fn unread_count(&self, recipient: &str) -> Result<i64, NotificationError> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint FROM notifications
            WHERE lower(recipient_handle) = lower($1) AND read_at IS NULL
            "#,
        )
        .bind(recipient)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    async fn mark_read(&self, recipient: &str, ids: &[Uuid]) -> Result<u64, NotificationError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let res = sqlx::query(
            r#"
            UPDATE notifications SET read_at = NOW()
            WHERE lower(recipient_handle) = lower($1)
              AND id = ANY($2)
              AND read_at IS NULL
            "#,
        )
        .bind(recipient)
        .bind(ids)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    async fn mark_all_read(&self, recipient: &str) -> Result<u64, NotificationError> {
        let res = sqlx::query(
            r#"
            UPDATE notifications SET read_at = NOW()
            WHERE lower(recipient_handle) = lower($1) AND read_at IS NULL
            "#,
        )
        .bind(recipient)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    async fn delete_from_actor(
        &self,
        recipient: &str,
        actor: &str,
    ) -> Result<u64, NotificationError> {
        let res = sqlx::query(
            r#"
            DELETE FROM notifications
            WHERE lower(recipient_handle) = lower($1) AND lower(actor_handle) = lower($2)
            "#,
        )
        .bind(recipient)
        .bind(actor)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    async fn purge_older_than(&self, before: DateTime<Utc>) -> Result<u64, NotificationError> {
        let res = sqlx::query("DELETE FROM notifications WHERE created_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryNotificationStore {
        rows: Mutex<Vec<(String, Notification)>>,
    }

    impl MemoryNotificationStore {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn all_for(&self, recipient: &str) -> Vec<Notification> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .filter(|(r, _)| r.eq_ignore_ascii_case(recipient))
                .map(|(_, n)| n.clone())
                .collect()
        }

        /// Backdate a row so retention and `since` can be tested
        /// without sleeping.
        pub fn set_created_at(&self, id: Uuid, at: DateTime<Utc>) {
            for (_, n) in self.rows.lock().unwrap().iter_mut() {
                if n.id == id {
                    n.created_at = at;
                }
            }
        }
    }

    #[async_trait]
    impl NotificationStore for MemoryNotificationStore {
        async fn create(
            &self,
            recipient: &str,
            kind: NotificationKind,
            actor: Option<&str>,
            payload: serde_json::Value,
        ) -> Result<Notification, NotificationError> {
            let n = Notification {
                id: Uuid::now_v7(),
                kind,
                actor_handle: actor.map(str::to_string),
                payload,
                created_at: Utc::now(),
                read_at: None,
            };
            self.rows
                .lock()
                .unwrap()
                .push((recipient.to_string(), n.clone()));
            Ok(n)
        }

        async fn list(
            &self,
            recipient: &str,
            since: Option<DateTime<Utc>>,
            limit: i64,
        ) -> Result<Vec<Notification>, NotificationError> {
            let limit = limit.clamp(1, PAGE_LIMIT_MAX) as usize;
            let mut out: Vec<Notification> = self
                .all_for(recipient)
                .into_iter()
                .filter(|n| since.map(|s| n.created_at > s).unwrap_or(true))
                .collect();
            out.sort_by_key(|n| std::cmp::Reverse(n.created_at));
            out.truncate(limit);
            Ok(out)
        }

        async fn unread_count(&self, recipient: &str) -> Result<i64, NotificationError> {
            Ok(self
                .all_for(recipient)
                .iter()
                .filter(|n| n.read_at.is_none())
                .count() as i64)
        }

        async fn mark_read(&self, recipient: &str, ids: &[Uuid]) -> Result<u64, NotificationError> {
            let mut n = 0;
            for (r, row) in self.rows.lock().unwrap().iter_mut() {
                if r.eq_ignore_ascii_case(recipient)
                    && ids.contains(&row.id)
                    && row.read_at.is_none()
                {
                    row.read_at = Some(Utc::now());
                    n += 1;
                }
            }
            Ok(n)
        }

        async fn mark_all_read(&self, recipient: &str) -> Result<u64, NotificationError> {
            let mut n = 0;
            for (r, row) in self.rows.lock().unwrap().iter_mut() {
                if r.eq_ignore_ascii_case(recipient) && row.read_at.is_none() {
                    row.read_at = Some(Utc::now());
                    n += 1;
                }
            }
            Ok(n)
        }

        async fn delete_from_actor(
            &self,
            recipient: &str,
            actor: &str,
        ) -> Result<u64, NotificationError> {
            let mut g = self.rows.lock().unwrap();
            let before = g.len();
            g.retain(|(r, n)| {
                !(r.eq_ignore_ascii_case(recipient)
                    && n.actor_handle
                        .as_deref()
                        .map(|a| a.eq_ignore_ascii_case(actor))
                        .unwrap_or(false))
            });
            Ok((before - g.len()) as u64)
        }

        async fn purge_older_than(&self, before: DateTime<Utc>) -> Result<u64, NotificationError> {
            let mut g = self.rows.lock().unwrap();
            let n = g.len();
            g.retain(|(_, row)| row.created_at >= before);
            Ok((n - g.len()) as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryNotificationStore;
    use super::*;
    use serde_json::json;

    #[test]
    fn kind_round_trips() {
        for k in [
            NotificationKind::FriendRequest,
            NotificationKind::FriendAccepted,
        ] {
            assert_eq!(NotificationKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(NotificationKind::parse("bogus"), None);
    }

    #[tokio::test]
    async fn list_is_per_recipient_newest_first_with_since() {
        let s = MemoryNotificationStore::new();
        let a = s
            .create(
                "alice",
                NotificationKind::FriendRequest,
                Some("bob"),
                json!({}),
            )
            .await
            .unwrap();
        let b = s
            .create(
                "Alice",
                NotificationKind::FriendAccepted,
                Some("carol"),
                json!({}),
            )
            .await
            .unwrap();
        s.create("bob", NotificationKind::FriendRequest, Some("x"), json!({}))
            .await
            .unwrap();
        let t0 = Utc::now() - Duration::minutes(10);
        s.set_created_at(a.id, t0);

        let all = s.list("alice", None, 50).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, b.id, "newest first");

        let newer = s.list("alice", Some(t0), 50).await.unwrap();
        assert_eq!(newer.len(), 1, "since is strictly after");
        assert_eq!(newer[0].id, b.id);
    }

    #[tokio::test]
    async fn mark_read_only_touches_own_rows() {
        let s = MemoryNotificationStore::new();
        let mine = s
            .create(
                "alice",
                NotificationKind::FriendRequest,
                Some("bob"),
                json!({}),
            )
            .await
            .unwrap();
        let theirs = s
            .create(
                "bob",
                NotificationKind::FriendRequest,
                Some("alice"),
                json!({}),
            )
            .await
            .unwrap();
        assert_eq!(s.unread_count("alice").await.unwrap(), 1);
        let n = s.mark_read("alice", &[mine.id, theirs.id]).await.unwrap();
        assert_eq!(n, 1, "someone else's id is ignored");
        assert_eq!(s.unread_count("alice").await.unwrap(), 0);
        assert_eq!(s.unread_count("bob").await.unwrap(), 1);
        assert_eq!(s.mark_read("alice", &[mine.id]).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn mark_all_read_and_delete_from_actor() {
        let s = MemoryNotificationStore::new();
        s.create(
            "alice",
            NotificationKind::FriendRequest,
            Some("troll"),
            json!({}),
        )
        .await
        .unwrap();
        s.create(
            "alice",
            NotificationKind::FriendRequest,
            Some("pal"),
            json!({}),
        )
        .await
        .unwrap();
        assert_eq!(s.delete_from_actor("ALICE", "Troll").await.unwrap(), 1);
        assert_eq!(s.list("alice", None, 50).await.unwrap().len(), 1);
        assert_eq!(s.mark_all_read("alice").await.unwrap(), 1);
        assert_eq!(s.unread_count("alice").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn purge_removes_only_old_rows() {
        let s = MemoryNotificationStore::new();
        let old = s
            .create("alice", NotificationKind::FriendRequest, None, json!({}))
            .await
            .unwrap();
        s.create("alice", NotificationKind::FriendRequest, None, json!({}))
            .await
            .unwrap();
        s.set_created_at(old.id, Utc::now() - Duration::days(RETENTION_DAYS + 1));
        let cutoff = Utc::now() - retention_window();
        assert_eq!(s.purge_older_than(cutoff).await.unwrap(), 1);
        assert_eq!(s.list("alice", None, 50).await.unwrap().len(), 1);
    }
}
