//! Friends, blocks and mutes (social phase 1).
//!
//! A friendship is mutual and consented to: it starts as a
//! `friend_requests` row in `pending`, and only an accept writes the
//! `friendships` row. It is deliberately separate from sharing — being
//! someone's friend grants no view of their stats. Sharing with friends
//! is a later, explicit choice (phase 1b).
//!
//! Everything is keyed by handle and compared case-insensitively, the
//! same convention as `share_metadata` / `share_reports`. Handles are
//! stored in the case the user claimed them (the handler resolves the
//! canonical `users.claimed_handle` before writing), so lists render the
//! name as its owner writes it.
//!
//! Backed by migration 0070. Status and policy vocabularies are closed
//! here and stored as TEXT.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

/// Sender-side cap on friend requests. Generous for a real player
/// adding a crew after a session, low enough that a script working
/// through the discover list stops quickly.
pub const REQUEST_RATE_LIMIT_WINDOW_HOURS: i64 = 24;
pub const REQUEST_RATE_LIMIT_PER_WINDOW: i64 = 30;

pub fn request_rate_limit_window() -> Duration {
    Duration::hours(REQUEST_RATE_LIMIT_WINDOW_HOURS)
}

/// Upper bound on list reads. A friends list past this is not a
/// realistic player, and an unbounded read is a cheap way to make the
/// server do a lot of work.
pub const LIST_LIMIT: i64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestStatus {
    Pending,
    Accepted,
    Declined,
    Cancelled,
}

impl FriendRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "accepted" => Self::Accepted,
            "declined" => Self::Declined,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// Who may send this user a friend request. `NULL` in the column reads
/// as `Everyone`, so existing users need no backfill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestPolicy {
    Everyone,
    Nobody,
}

impl FriendRequestPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Everyone => "everyone",
            Self::Nobody => "nobody",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "everyone" => Self::Everyone,
            "nobody" => Self::Nobody,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FriendRequest {
    pub id: Uuid,
    pub requester_handle: String,
    pub recipient_handle: String,
    pub status: FriendRequestStatus,
    pub created_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Friend {
    pub handle: String,
    pub since: DateTime<Utc>,
    /// Whether this handle is proven to be the player's RSI handle.
    /// Clients offer "copy handle" / "add in game" only when true:
    /// copying an unproven handle would send an in-game invite to
    /// whoever really owns that name.
    pub rsi_verified: bool,
}

/// One entry on a block or mute list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListedHandle {
    pub handle: String,
    pub since: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum SocialError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("request not found")]
    NotFound,
    #[error("request is no longer pending")]
    NotPending,
    #[error("a pending request already exists")]
    DuplicatePending,
    #[error("stored value out of domain: {0}")]
    Domain(String),
}

#[async_trait]
pub trait SocialStore: Send + Sync + 'static {
    async fn get_policy(&self, handle: &str) -> Result<FriendRequestPolicy, SocialError>;
    async fn set_policy(
        &self,
        handle: &str,
        policy: FriendRequestPolicy,
    ) -> Result<(), SocialError>;

    /// Create a `pending` request. `DuplicatePending` when one already
    /// exists in the same direction.
    async fn create_request(
        &self,
        requester: &str,
        recipient: &str,
    ) -> Result<FriendRequest, SocialError>;
    async fn get_request(&self, id: Uuid) -> Result<Option<FriendRequest>, SocialError>;
    async fn find_pending(
        &self,
        requester: &str,
        recipient: &str,
    ) -> Result<Option<FriendRequest>, SocialError>;
    /// Move a `pending` request to `to`. `NotPending` if it already
    /// moved, `NotFound` if it never existed — a double-click accept
    /// must not write a second friendship.
    async fn resolve_request(
        &self,
        id: Uuid,
        to: FriendRequestStatus,
    ) -> Result<FriendRequest, SocialError>;
    /// Pending requests addressed to `handle`, excluding those from
    /// anyone `handle` has blocked, newest first.
    async fn list_incoming(&self, handle: &str) -> Result<Vec<FriendRequest>, SocialError>;
    /// Pending requests `handle` has sent, newest first.
    async fn list_outgoing(&self, handle: &str) -> Result<Vec<FriendRequest>, SocialError>;
    /// Cancel every pending request between the two, either direction.
    async fn cancel_pending_between(&self, a: &str, b: &str) -> Result<u64, SocialError>;
    async fn count_recent_requests(
        &self,
        requester: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, SocialError>;

    /// Idempotent: an existing friendship is left as it is.
    async fn add_friendship(&self, a: &str, b: &str) -> Result<(), SocialError>;
    /// `true` when a friendship existed and was removed.
    async fn remove_friendship(&self, a: &str, b: &str) -> Result<bool, SocialError>;
    async fn are_friends(&self, a: &str, b: &str) -> Result<bool, SocialError>;
    async fn list_friends(&self, handle: &str) -> Result<Vec<Friend>, SocialError>;

    /// Idempotent.
    async fn block(&self, blocker: &str, blocked: &str) -> Result<(), SocialError>;
    async fn unblock(&self, blocker: &str, blocked: &str) -> Result<bool, SocialError>;
    async fn is_blocked(&self, blocker: &str, blocked: &str) -> Result<bool, SocialError>;
    async fn list_blocks(&self, blocker: &str) -> Result<Vec<ListedHandle>, SocialError>;

    /// Idempotent.
    async fn mute(&self, muter: &str, muted: &str) -> Result<(), SocialError>;
    async fn unmute(&self, muter: &str, muted: &str) -> Result<bool, SocialError>;
    async fn is_muted(&self, muter: &str, muted: &str) -> Result<bool, SocialError>;
    async fn list_mutes(&self, muter: &str) -> Result<Vec<ListedHandle>, SocialError>;
}

/// Order a pair the way `friendships` stores it: lower-cased
/// comparison, original case kept.
fn ordered_pair<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    if a.to_ascii_lowercase() <= b.to_ascii_lowercase() {
        (a, b)
    } else {
        (b, a)
    }
}

/// Erase every social row that names `handle`, on either side, inside
/// the caller's account-deletion transaction. Without this a deleted
/// account's friendships and blocks would survive, and a later sign-up
/// under the same handle would inherit them.
pub async fn delete_social_rows_for(
    conn: &mut sqlx::PgConnection,
    handle: &str,
) -> Result<(), sqlx::Error> {
    // Fixed literal (table, column) pairs, never caller input; the
    // handle is still bound.
    for (table, column) in [
        ("friend_requests", "requester_handle"),
        ("friend_requests", "recipient_handle"),
        ("friendships", "handle_a"),
        ("friendships", "handle_b"),
        ("user_blocks", "blocker_handle"),
        ("user_blocks", "blocked_handle"),
        ("user_mutes", "muter_handle"),
        ("user_mutes", "muted_handle"),
        ("notifications", "recipient_handle"),
        ("notifications", "actor_handle"),
    ] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE lower({column}) = lower($1)"
        ))
        .bind(handle)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub struct PostgresSocialStore {
    pool: PgPool,
}

impl PostgresSocialStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type RequestRow = (
    Uuid,
    String,
    String,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

fn row_to_request(row: RequestRow) -> Result<FriendRequest, SocialError> {
    let status = FriendRequestStatus::parse(&row.3)
        .ok_or_else(|| SocialError::Domain(format!("status={}", row.3)))?;
    Ok(FriendRequest {
        id: row.0,
        requester_handle: row.1,
        recipient_handle: row.2,
        status,
        created_at: row.4,
        responded_at: row.5,
    })
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}

#[async_trait]
impl SocialStore for PostgresSocialStore {
    async fn get_policy(&self, handle: &str) -> Result<FriendRequestPolicy, SocialError> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT friend_request_policy FROM users WHERE lower(claimed_handle) = lower($1)",
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        match row.and_then(|(p,)| p) {
            None => Ok(FriendRequestPolicy::Everyone),
            Some(p) => FriendRequestPolicy::parse(&p)
                .ok_or_else(|| SocialError::Domain(format!("policy={p}"))),
        }
    }

    async fn set_policy(
        &self,
        handle: &str,
        policy: FriendRequestPolicy,
    ) -> Result<(), SocialError> {
        sqlx::query(
            "UPDATE users SET friend_request_policy = $2 WHERE lower(claimed_handle) = lower($1)",
        )
        .bind(handle)
        .bind(policy.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn create_request(
        &self,
        requester: &str,
        recipient: &str,
    ) -> Result<FriendRequest, SocialError> {
        let res: Result<RequestRow, sqlx::Error> = sqlx::query_as(
            r#"
            INSERT INTO friend_requests (requester_handle, recipient_handle)
            VALUES ($1, $2)
            RETURNING id, requester_handle, recipient_handle, status, created_at, responded_at
            "#,
        )
        .bind(requester)
        .bind(recipient)
        .fetch_one(&self.pool)
        .await;
        match res {
            Ok(row) => row_to_request(row),
            Err(e) if is_unique_violation(&e) => Err(SocialError::DuplicatePending),
            Err(e) => Err(e.into()),
        }
    }

    async fn get_request(&self, id: Uuid) -> Result<Option<FriendRequest>, SocialError> {
        let row: Option<RequestRow> = sqlx::query_as(
            r#"
            SELECT id, requester_handle, recipient_handle, status, created_at, responded_at
            FROM friend_requests WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_request).transpose()
    }

    async fn find_pending(
        &self,
        requester: &str,
        recipient: &str,
    ) -> Result<Option<FriendRequest>, SocialError> {
        let row: Option<RequestRow> = sqlx::query_as(
            r#"
            SELECT id, requester_handle, recipient_handle, status, created_at, responded_at
            FROM friend_requests
            WHERE lower(requester_handle) = lower($1)
              AND lower(recipient_handle) = lower($2)
              AND status = 'pending'
            "#,
        )
        .bind(requester)
        .bind(recipient)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_request).transpose()
    }

    async fn resolve_request(
        &self,
        id: Uuid,
        to: FriendRequestStatus,
    ) -> Result<FriendRequest, SocialError> {
        // Guarded on status = 'pending' so a stale second call updates
        // nothing, which is then told apart from a missing id.
        let row: Option<RequestRow> = sqlx::query_as(
            r#"
            UPDATE friend_requests
            SET status = $2, responded_at = NOW()
            WHERE id = $1 AND status = 'pending'
            RETURNING id, requester_handle, recipient_handle, status, created_at, responded_at
            "#,
        )
        .bind(id)
        .bind(to.as_str())
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(r) => row_to_request(r),
            None => match self.get_request(id).await? {
                Some(_) => Err(SocialError::NotPending),
                None => Err(SocialError::NotFound),
            },
        }
    }

    async fn list_incoming(&self, handle: &str) -> Result<Vec<FriendRequest>, SocialError> {
        let rows: Vec<RequestRow> = sqlx::query_as(
            r#"
            SELECT r.id, r.requester_handle, r.recipient_handle, r.status,
                   r.created_at, r.responded_at
            FROM friend_requests r
            WHERE lower(r.recipient_handle) = lower($1)
              AND r.status = 'pending'
              AND NOT EXISTS (
                  SELECT 1 FROM user_blocks b
                  WHERE lower(b.blocker_handle) = lower($1)
                    AND lower(b.blocked_handle) = lower(r.requester_handle)
              )
            ORDER BY r.created_at DESC
            LIMIT $2
            "#,
        )
        .bind(handle)
        .bind(LIST_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_request).collect()
    }

    async fn list_outgoing(&self, handle: &str) -> Result<Vec<FriendRequest>, SocialError> {
        let rows: Vec<RequestRow> = sqlx::query_as(
            r#"
            SELECT id, requester_handle, recipient_handle, status, created_at, responded_at
            FROM friend_requests
            WHERE lower(requester_handle) = lower($1) AND status = 'pending'
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(handle)
        .bind(LIST_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_request).collect()
    }

    async fn cancel_pending_between(&self, a: &str, b: &str) -> Result<u64, SocialError> {
        let res = sqlx::query(
            r#"
            UPDATE friend_requests
            SET status = 'cancelled', responded_at = NOW()
            WHERE status = 'pending'
              AND ((lower(requester_handle) = lower($1) AND lower(recipient_handle) = lower($2))
                OR (lower(requester_handle) = lower($2) AND lower(recipient_handle) = lower($1)))
            "#,
        )
        .bind(a)
        .bind(b)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    async fn count_recent_requests(
        &self,
        requester: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, SocialError> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint FROM friend_requests
            WHERE lower(requester_handle) = lower($1) AND created_at >= $2
            "#,
        )
        .bind(requester)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    async fn add_friendship(&self, a: &str, b: &str) -> Result<(), SocialError> {
        let (x, y) = ordered_pair(a, b);
        sqlx::query(
            r#"
            INSERT INTO friendships (handle_a, handle_b) VALUES ($1, $2)
            ON CONFLICT (lower(handle_a), lower(handle_b)) DO NOTHING
            "#,
        )
        .bind(x)
        .bind(y)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_friendship(&self, a: &str, b: &str) -> Result<bool, SocialError> {
        let (x, y) = ordered_pair(a, b);
        let res = sqlx::query(
            "DELETE FROM friendships WHERE lower(handle_a) = lower($1) AND lower(handle_b) = lower($2)",
        )
        .bind(x)
        .bind(y)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn are_friends(&self, a: &str, b: &str) -> Result<bool, SocialError> {
        let (x, y) = ordered_pair(a, b);
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM friendships WHERE lower(handle_a) = lower($1) AND lower(handle_b) = lower($2)",
        )
        .bind(x)
        .bind(y)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    async fn list_friends(&self, handle: &str) -> Result<Vec<Friend>, SocialError> {
        let rows: Vec<(String, DateTime<Utc>, bool)> = sqlx::query_as(
            r#"
            SELECT f.other, f.created_at, (u.rsi_verified_at IS NOT NULL) AS verified
            FROM (
                SELECT CASE WHEN lower(handle_a) = lower($1) THEN handle_b ELSE handle_a END
                           AS other,
                       created_at
                FROM friendships
                WHERE lower(handle_a) = lower($1) OR lower(handle_b) = lower($1)
            ) f
            LEFT JOIN users u ON lower(u.claimed_handle) = lower(f.other)
            ORDER BY lower(f.other)
            LIMIT $2
            "#,
        )
        .bind(handle)
        .bind(LIST_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(handle, since, rsi_verified)| Friend {
                handle,
                since,
                rsi_verified,
            })
            .collect())
    }

    async fn block(&self, blocker: &str, blocked: &str) -> Result<(), SocialError> {
        sqlx::query(
            r#"
            INSERT INTO user_blocks (blocker_handle, blocked_handle) VALUES ($1, $2)
            ON CONFLICT (lower(blocker_handle), lower(blocked_handle)) DO NOTHING
            "#,
        )
        .bind(blocker)
        .bind(blocked)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn unblock(&self, blocker: &str, blocked: &str) -> Result<bool, SocialError> {
        let res = sqlx::query(
            "DELETE FROM user_blocks WHERE lower(blocker_handle) = lower($1) AND lower(blocked_handle) = lower($2)",
        )
        .bind(blocker)
        .bind(blocked)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn is_blocked(&self, blocker: &str, blocked: &str) -> Result<bool, SocialError> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM user_blocks WHERE lower(blocker_handle) = lower($1) AND lower(blocked_handle) = lower($2)",
        )
        .bind(blocker)
        .bind(blocked)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    async fn list_blocks(&self, blocker: &str) -> Result<Vec<ListedHandle>, SocialError> {
        let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            r#"
            SELECT blocked_handle, created_at FROM user_blocks
            WHERE lower(blocker_handle) = lower($1)
            ORDER BY lower(blocked_handle)
            LIMIT $2
            "#,
        )
        .bind(blocker)
        .bind(LIST_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(handle, since)| ListedHandle { handle, since })
            .collect())
    }

    async fn mute(&self, muter: &str, muted: &str) -> Result<(), SocialError> {
        sqlx::query(
            r#"
            INSERT INTO user_mutes (muter_handle, muted_handle) VALUES ($1, $2)
            ON CONFLICT (lower(muter_handle), lower(muted_handle)) DO NOTHING
            "#,
        )
        .bind(muter)
        .bind(muted)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn unmute(&self, muter: &str, muted: &str) -> Result<bool, SocialError> {
        let res = sqlx::query(
            "DELETE FROM user_mutes WHERE lower(muter_handle) = lower($1) AND lower(muted_handle) = lower($2)",
        )
        .bind(muter)
        .bind(muted)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn is_muted(&self, muter: &str, muted: &str) -> Result<bool, SocialError> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM user_mutes WHERE lower(muter_handle) = lower($1) AND lower(muted_handle) = lower($2)",
        )
        .bind(muter)
        .bind(muted)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    async fn list_mutes(&self, muter: &str) -> Result<Vec<ListedHandle>, SocialError> {
        let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            r#"
            SELECT muted_handle, created_at FROM user_mutes
            WHERE lower(muter_handle) = lower($1)
            ORDER BY lower(muted_handle)
            LIMIT $2
            "#,
        )
        .bind(muter)
        .bind(LIST_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(handle, since)| ListedHandle { handle, since })
            .collect())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    fn eq(a: &str, b: &str) -> bool {
        a.eq_ignore_ascii_case(b)
    }

    #[derive(Default)]
    struct Inner {
        policies: HashMap<String, FriendRequestPolicy>,
        requests: Vec<FriendRequest>,
        friendships: Vec<(String, String, DateTime<Utc>)>,
        blocks: Vec<(String, String, DateTime<Utc>)>,
        mutes: Vec<(String, String, DateTime<Utc>)>,
        verified: HashSet<String>,
    }

    #[derive(Default)]
    pub struct MemorySocialStore {
        inner: Mutex<Inner>,
    }

    impl MemorySocialStore {
        pub fn new() -> Self {
            Self::default()
        }

        /// Stand-in for `users.rsi_verified_at`, which the Postgres
        /// impl reads through a join.
        pub fn mark_verified(&self, handle: &str) {
            self.inner
                .lock()
                .unwrap()
                .verified
                .insert(handle.to_ascii_lowercase());
        }
    }

    #[async_trait]
    impl SocialStore for MemorySocialStore {
        async fn get_policy(&self, handle: &str) -> Result<FriendRequestPolicy, SocialError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .policies
                .get(&handle.to_ascii_lowercase())
                .copied()
                .unwrap_or(FriendRequestPolicy::Everyone))
        }

        async fn set_policy(
            &self,
            handle: &str,
            policy: FriendRequestPolicy,
        ) -> Result<(), SocialError> {
            self.inner
                .lock()
                .unwrap()
                .policies
                .insert(handle.to_ascii_lowercase(), policy);
            Ok(())
        }

        async fn create_request(
            &self,
            requester: &str,
            recipient: &str,
        ) -> Result<FriendRequest, SocialError> {
            let mut g = self.inner.lock().unwrap();
            if g.requests.iter().any(|r| {
                r.status == FriendRequestStatus::Pending
                    && eq(&r.requester_handle, requester)
                    && eq(&r.recipient_handle, recipient)
            }) {
                return Err(SocialError::DuplicatePending);
            }
            let r = FriendRequest {
                id: Uuid::now_v7(),
                requester_handle: requester.into(),
                recipient_handle: recipient.into(),
                status: FriendRequestStatus::Pending,
                created_at: Utc::now(),
                responded_at: None,
            };
            g.requests.push(r.clone());
            Ok(r)
        }

        async fn get_request(&self, id: Uuid) -> Result<Option<FriendRequest>, SocialError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .requests
                .iter()
                .find(|r| r.id == id)
                .cloned())
        }

        async fn find_pending(
            &self,
            requester: &str,
            recipient: &str,
        ) -> Result<Option<FriendRequest>, SocialError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .requests
                .iter()
                .find(|r| {
                    r.status == FriendRequestStatus::Pending
                        && eq(&r.requester_handle, requester)
                        && eq(&r.recipient_handle, recipient)
                })
                .cloned())
        }

        async fn resolve_request(
            &self,
            id: Uuid,
            to: FriendRequestStatus,
        ) -> Result<FriendRequest, SocialError> {
            let mut g = self.inner.lock().unwrap();
            let r = g
                .requests
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or(SocialError::NotFound)?;
            if r.status != FriendRequestStatus::Pending {
                return Err(SocialError::NotPending);
            }
            r.status = to;
            r.responded_at = Some(Utc::now());
            Ok(r.clone())
        }

        async fn list_incoming(&self, handle: &str) -> Result<Vec<FriendRequest>, SocialError> {
            let g = self.inner.lock().unwrap();
            let mut out: Vec<FriendRequest> = g
                .requests
                .iter()
                .filter(|r| {
                    r.status == FriendRequestStatus::Pending
                        && eq(&r.recipient_handle, handle)
                        && !g
                            .blocks
                            .iter()
                            .any(|(br, bd, _)| eq(br, handle) && eq(bd, &r.requester_handle))
                })
                .cloned()
                .collect();
            out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
            Ok(out)
        }

        async fn list_outgoing(&self, handle: &str) -> Result<Vec<FriendRequest>, SocialError> {
            let g = self.inner.lock().unwrap();
            let mut out: Vec<FriendRequest> = g
                .requests
                .iter()
                .filter(|r| {
                    r.status == FriendRequestStatus::Pending && eq(&r.requester_handle, handle)
                })
                .cloned()
                .collect();
            out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
            Ok(out)
        }

        async fn cancel_pending_between(&self, a: &str, b: &str) -> Result<u64, SocialError> {
            let mut g = self.inner.lock().unwrap();
            let mut n = 0;
            for r in g.requests.iter_mut() {
                let between = (eq(&r.requester_handle, a) && eq(&r.recipient_handle, b))
                    || (eq(&r.requester_handle, b) && eq(&r.recipient_handle, a));
                if between && r.status == FriendRequestStatus::Pending {
                    r.status = FriendRequestStatus::Cancelled;
                    r.responded_at = Some(Utc::now());
                    n += 1;
                }
            }
            Ok(n)
        }

        async fn count_recent_requests(
            &self,
            requester: &str,
            since: DateTime<Utc>,
        ) -> Result<i64, SocialError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .requests
                .iter()
                .filter(|r| eq(&r.requester_handle, requester) && r.created_at >= since)
                .count() as i64)
        }

        async fn add_friendship(&self, a: &str, b: &str) -> Result<(), SocialError> {
            let mut g = self.inner.lock().unwrap();
            let (x, y) = ordered_pair(a, b);
            if !g.friendships.iter().any(|(p, q, _)| eq(p, x) && eq(q, y)) {
                g.friendships.push((x.into(), y.into(), Utc::now()));
            }
            Ok(())
        }

        async fn remove_friendship(&self, a: &str, b: &str) -> Result<bool, SocialError> {
            let mut g = self.inner.lock().unwrap();
            let (x, y) = ordered_pair(a, b);
            let before = g.friendships.len();
            g.friendships.retain(|(p, q, _)| !(eq(p, x) && eq(q, y)));
            Ok(g.friendships.len() < before)
        }

        async fn are_friends(&self, a: &str, b: &str) -> Result<bool, SocialError> {
            let g = self.inner.lock().unwrap();
            let (x, y) = ordered_pair(a, b);
            Ok(g.friendships.iter().any(|(p, q, _)| eq(p, x) && eq(q, y)))
        }

        async fn list_friends(&self, handle: &str) -> Result<Vec<Friend>, SocialError> {
            let g = self.inner.lock().unwrap();
            let mut out: Vec<Friend> = g
                .friendships
                .iter()
                .filter_map(|(p, q, at)| {
                    let other = if eq(p, handle) {
                        q
                    } else if eq(q, handle) {
                        p
                    } else {
                        return None;
                    };
                    Some(Friend {
                        handle: other.clone(),
                        since: *at,
                        rsi_verified: g.verified.contains(&other.to_ascii_lowercase()),
                    })
                })
                .collect();
            out.sort_by_key(|f| f.handle.to_ascii_lowercase());
            Ok(out)
        }

        async fn block(&self, blocker: &str, blocked: &str) -> Result<(), SocialError> {
            let mut g = self.inner.lock().unwrap();
            if !g
                .blocks
                .iter()
                .any(|(a, b, _)| eq(a, blocker) && eq(b, blocked))
            {
                g.blocks.push((blocker.into(), blocked.into(), Utc::now()));
            }
            Ok(())
        }

        async fn unblock(&self, blocker: &str, blocked: &str) -> Result<bool, SocialError> {
            let mut g = self.inner.lock().unwrap();
            let before = g.blocks.len();
            g.blocks
                .retain(|(a, b, _)| !(eq(a, blocker) && eq(b, blocked)));
            Ok(g.blocks.len() < before)
        }

        async fn is_blocked(&self, blocker: &str, blocked: &str) -> Result<bool, SocialError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .blocks
                .iter()
                .any(|(a, b, _)| eq(a, blocker) && eq(b, blocked)))
        }

        async fn list_blocks(&self, blocker: &str) -> Result<Vec<ListedHandle>, SocialError> {
            let g = self.inner.lock().unwrap();
            let mut out: Vec<ListedHandle> = g
                .blocks
                .iter()
                .filter(|(a, _, _)| eq(a, blocker))
                .map(|(_, b, at)| ListedHandle {
                    handle: b.clone(),
                    since: *at,
                })
                .collect();
            out.sort_by_key(|h| h.handle.to_ascii_lowercase());
            Ok(out)
        }

        async fn mute(&self, muter: &str, muted: &str) -> Result<(), SocialError> {
            let mut g = self.inner.lock().unwrap();
            if !g.mutes.iter().any(|(a, b, _)| eq(a, muter) && eq(b, muted)) {
                g.mutes.push((muter.into(), muted.into(), Utc::now()));
            }
            Ok(())
        }

        async fn unmute(&self, muter: &str, muted: &str) -> Result<bool, SocialError> {
            let mut g = self.inner.lock().unwrap();
            let before = g.mutes.len();
            g.mutes.retain(|(a, b, _)| !(eq(a, muter) && eq(b, muted)));
            Ok(g.mutes.len() < before)
        }

        async fn is_muted(&self, muter: &str, muted: &str) -> Result<bool, SocialError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .mutes
                .iter()
                .any(|(a, b, _)| eq(a, muter) && eq(b, muted)))
        }

        async fn list_mutes(&self, muter: &str) -> Result<Vec<ListedHandle>, SocialError> {
            let g = self.inner.lock().unwrap();
            let mut out: Vec<ListedHandle> = g
                .mutes
                .iter()
                .filter(|(a, _, _)| eq(a, muter))
                .map(|(_, b, at)| ListedHandle {
                    handle: b.clone(),
                    since: *at,
                })
                .collect();
            out.sort_by_key(|h| h.handle.to_ascii_lowercase());
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemorySocialStore;
    use super::*;

    #[test]
    fn vocabularies_round_trip() {
        for s in [
            FriendRequestStatus::Pending,
            FriendRequestStatus::Accepted,
            FriendRequestStatus::Declined,
            FriendRequestStatus::Cancelled,
        ] {
            assert_eq!(FriendRequestStatus::parse(s.as_str()), Some(s));
        }
        for p in [FriendRequestPolicy::Everyone, FriendRequestPolicy::Nobody] {
            assert_eq!(FriendRequestPolicy::parse(p.as_str()), Some(p));
        }
        assert_eq!(FriendRequestStatus::parse("bogus"), None);
        assert_eq!(FriendRequestPolicy::parse("org"), None);
    }

    #[test]
    fn ordered_pair_is_case_insensitive_and_stable() {
        assert_eq!(ordered_pair("bob", "Alice"), ("Alice", "bob"));
        assert_eq!(ordered_pair("Alice", "bob"), ("Alice", "bob"));
    }

    #[tokio::test]
    async fn one_pending_request_per_direction() {
        let s = MemorySocialStore::new();
        s.create_request("alice", "bob").await.unwrap();
        let again = s.create_request("Alice", "BOB").await;
        assert!(matches!(again, Err(SocialError::DuplicatePending)));
        // The reverse direction is a different request.
        s.create_request("bob", "alice").await.unwrap();
    }

    #[tokio::test]
    async fn resolve_moves_pending_once_only() {
        let s = MemorySocialStore::new();
        let r = s.create_request("alice", "bob").await.unwrap();
        let done = s
            .resolve_request(r.id, FriendRequestStatus::Accepted)
            .await
            .unwrap();
        assert_eq!(done.status, FriendRequestStatus::Accepted);
        assert!(done.responded_at.is_some());
        let twice = s.resolve_request(r.id, FriendRequestStatus::Declined).await;
        assert!(matches!(twice, Err(SocialError::NotPending)));
        let missing = s
            .resolve_request(Uuid::now_v7(), FriendRequestStatus::Accepted)
            .await;
        assert!(matches!(missing, Err(SocialError::NotFound)));
        // Once resolved, the direction is free for a new request.
        s.create_request("alice", "bob").await.unwrap();
    }

    #[tokio::test]
    async fn friendship_is_symmetric_and_idempotent() {
        let s = MemorySocialStore::new();
        s.mark_verified("Bob");
        s.add_friendship("alice", "Bob").await.unwrap();
        s.add_friendship("bob", "ALICE").await.unwrap();
        assert!(s.are_friends("BOB", "alice").await.unwrap());
        let alice = s.list_friends("alice").await.unwrap();
        assert_eq!(alice.len(), 1, "second add must not duplicate");
        assert_eq!(alice[0].handle, "Bob");
        assert!(alice[0].rsi_verified);
        let bob = s.list_friends("bob").await.unwrap();
        assert_eq!(bob[0].handle, "alice");
        assert!(!bob[0].rsi_verified);
        assert!(s.remove_friendship("Bob", "alice").await.unwrap());
        assert!(!s.are_friends("alice", "bob").await.unwrap());
        assert!(!s.remove_friendship("alice", "bob").await.unwrap());
    }

    #[tokio::test]
    async fn incoming_hides_requests_from_blocked_senders() {
        let s = MemorySocialStore::new();
        s.create_request("troll", "alice").await.unwrap();
        s.create_request("friend", "alice").await.unwrap();
        s.block("alice", "TROLL").await.unwrap();
        let incoming = s.list_incoming("alice").await.unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].requester_handle, "friend");
        // The sender still sees their own request as pending: a block
        // must look the same as being ignored.
        assert_eq!(s.list_outgoing("troll").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cancel_pending_between_covers_both_directions() {
        let s = MemorySocialStore::new();
        s.create_request("alice", "bob").await.unwrap();
        s.create_request("bob", "alice").await.unwrap();
        s.create_request("alice", "carol").await.unwrap();
        assert_eq!(s.cancel_pending_between("BOB", "alice").await.unwrap(), 2);
        assert!(s.find_pending("alice", "bob").await.unwrap().is_none());
        assert!(s.find_pending("alice", "carol").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn blocks_and_mutes_are_idempotent_and_directional() {
        let s = MemorySocialStore::new();
        s.block("alice", "bob").await.unwrap();
        s.block("Alice", "BOB").await.unwrap();
        assert_eq!(s.list_blocks("alice").await.unwrap().len(), 1);
        assert!(s.is_blocked("alice", "bob").await.unwrap());
        assert!(!s.is_blocked("bob", "alice").await.unwrap());
        assert!(s.unblock("alice", "bob").await.unwrap());
        assert!(!s.unblock("alice", "bob").await.unwrap());

        s.mute("alice", "bob").await.unwrap();
        s.mute("alice", "bob").await.unwrap();
        assert_eq!(s.list_mutes("alice").await.unwrap().len(), 1);
        assert!(s.is_muted("alice", "bob").await.unwrap());
        assert!(!s.is_muted("bob", "alice").await.unwrap());
        assert!(s.unmute("alice", "bob").await.unwrap());
    }

    #[tokio::test]
    async fn policy_defaults_to_everyone() {
        let s = MemorySocialStore::new();
        assert_eq!(
            s.get_policy("alice").await.unwrap(),
            FriendRequestPolicy::Everyone
        );
        s.set_policy("Alice", FriendRequestPolicy::Nobody)
            .await
            .unwrap();
        assert_eq!(
            s.get_policy("alice").await.unwrap(),
            FriendRequestPolicy::Nobody
        );
    }

    #[tokio::test]
    async fn rate_limit_count_is_per_sender_and_windowed() {
        let s = MemorySocialStore::new();
        let before = Utc::now() - Duration::hours(1);
        s.create_request("alice", "a").await.unwrap();
        s.create_request("ALICE", "b").await.unwrap();
        s.create_request("bob", "c").await.unwrap();
        assert_eq!(s.count_recent_requests("alice", before).await.unwrap(), 2);
        let later = Utc::now() + Duration::hours(1);
        assert_eq!(s.count_recent_requests("alice", later).await.unwrap(), 0);
    }
}
/// Postgres round trip for every `PostgresSocialStore` and
/// `PostgresNotificationStore` method, plus the account-deletion sweep.
/// The Memory impls above cannot catch a wrong column, an index that
/// `ON CONFLICT` cannot use, or a case-folding mistake in SQL, so this
/// is the test that does. Runs only when STARSTATS_TEST_DATABASE_URL
/// points at a real Postgres, the same gate as the other round-trip
/// tests in this crate.
#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::notifications::{NotificationKind, NotificationStore, PostgresNotificationStore};

    const A: &str = "SocialProbeA";
    const B: &str = "socialprobeb";
    const C: &str = "SocialProbeC";

    async fn clean(pool: &PgPool) {
        let mut conn = pool.acquire().await.unwrap();
        for h in [A, B, C] {
            delete_social_rows_for(&mut conn, h).await.unwrap();
            sqlx::query("DELETE FROM users WHERE lower(claimed_handle) = lower($1)")
                .bind(h)
                .execute(&mut *conn)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn postgres_social_and_notification_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres round-trip test");
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
        clean(&pool).await;

        for (h, verified) in [(A, true), (B, false), (C, false)] {
            sqlx::query(
                "INSERT INTO users (id, email, password_hash, claimed_handle, rsi_verified_at)
                 VALUES (gen_random_uuid(), $1, 'x', $2,
                         CASE WHEN $3::bool THEN NOW() ELSE NULL END)",
            )
            .bind(format!("{}@example.com", h.to_ascii_lowercase()))
            .bind(h)
            .bind(verified)
            .execute(&pool)
            .await
            .expect("seed user");
        }

        let s = PostgresSocialStore::new(pool.clone());

        // Policy: NULL reads as everyone; set round-trips.
        assert_eq!(
            s.get_policy(A).await.unwrap(),
            FriendRequestPolicy::Everyone
        );
        s.set_policy("socialprobea", FriendRequestPolicy::Nobody)
            .await
            .unwrap();
        assert_eq!(s.get_policy(A).await.unwrap(), FriendRequestPolicy::Nobody);

        // Requests: one pending per direction, case-insensitively.
        let r = s.create_request(B, A).await.unwrap();
        assert!(matches!(
            s.create_request("SOCIALPROBEB", "socialprobea").await,
            Err(SocialError::DuplicatePending)
        ));
        assert_eq!(s.find_pending(B, A).await.unwrap().unwrap().id, r.id);
        assert_eq!(s.list_incoming(A).await.unwrap().len(), 1);
        assert_eq!(s.list_outgoing(B).await.unwrap().len(), 1);
        assert_eq!(
            s.count_recent_requests(B, Utc::now() - Duration::hours(1))
                .await
                .unwrap(),
            1
        );
        let done = s
            .resolve_request(r.id, FriendRequestStatus::Accepted)
            .await
            .unwrap();
        assert_eq!(done.status, FriendRequestStatus::Accepted);
        assert!(matches!(
            s.resolve_request(r.id, FriendRequestStatus::Declined).await,
            Err(SocialError::NotPending)
        ));
        assert!(matches!(
            s.resolve_request(Uuid::now_v7(), FriendRequestStatus::Declined)
                .await,
            Err(SocialError::NotFound)
        ));

        // Friendships: ordered pair, idempotent, verified flag joined.
        s.add_friendship(B, A).await.unwrap();
        s.add_friendship(A, B).await.unwrap();
        assert!(s.are_friends("SOCIALPROBEA", B).await.unwrap());
        let fa = s.list_friends(A).await.unwrap();
        assert_eq!(fa.len(), 1);
        assert_eq!(fa[0].handle, B);
        assert!(!fa[0].rsi_verified);
        let fb = s.list_friends(B).await.unwrap();
        assert_eq!(fb[0].handle, A);
        assert!(fb[0].rsi_verified);

        // Blocks hide incoming requests; cancel covers both directions.
        s.create_request(C, A).await.unwrap();
        s.create_request(A, C).await.unwrap();
        s.block(A, C).await.unwrap();
        s.block("socialprobea", "SOCIALPROBEC").await.unwrap();
        assert_eq!(s.list_blocks(A).await.unwrap().len(), 1);
        assert!(s.is_blocked(A, C).await.unwrap());
        assert!(!s.is_blocked(C, A).await.unwrap());
        assert!(s.list_incoming(A).await.unwrap().is_empty());
        assert_eq!(s.cancel_pending_between(C, A).await.unwrap(), 2);
        assert!(s.unblock(A, C).await.unwrap());
        assert!(!s.unblock(A, C).await.unwrap());

        // Mutes.
        s.mute(A, B).await.unwrap();
        s.mute(A, B).await.unwrap();
        assert_eq!(s.list_mutes(A).await.unwrap().len(), 1);
        assert!(s.is_muted(A, B).await.unwrap());
        assert!(s.unmute(A, B).await.unwrap());

        assert!(s.remove_friendship(A, B).await.unwrap());
        assert!(!s.are_friends(A, B).await.unwrap());

        // Notifications.
        let n = PostgresNotificationStore::new(pool.clone());
        let first = n
            .create(
                A,
                NotificationKind::FriendRequest,
                Some(B),
                serde_json::json!({"k": 1}),
            )
            .await
            .unwrap();
        let t = first.created_at;
        n.create(
            "socialprobea",
            NotificationKind::FriendAccepted,
            Some(C),
            serde_json::json!({}),
        )
        .await
        .unwrap();
        let all = n.list(A, None, 50).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all[0].kind,
            NotificationKind::FriendAccepted,
            "newest first"
        );
        assert_eq!(all[1].payload["k"], 1);
        assert_eq!(n.list(A, Some(t), 50).await.unwrap().len(), 1);
        assert_eq!(n.unread_count(A).await.unwrap(), 2);
        assert_eq!(n.mark_read(B, &[first.id]).await.unwrap(), 0);
        assert_eq!(n.mark_read(A, &[first.id]).await.unwrap(), 1);
        assert_eq!(n.mark_all_read(A).await.unwrap(), 1);
        assert_eq!(n.unread_count(A).await.unwrap(), 0);
        assert_eq!(n.delete_from_actor(A, "SOCIALPROBEC").await.unwrap(), 1);
        assert_eq!(
            n.purge_older_than(Utc::now() - Duration::days(1))
                .await
                .unwrap(),
            0,
            "nothing here is a day old"
        );

        // Account-deletion sweep leaves nothing that names the handle.
        s.add_friendship(A, C).await.unwrap();
        s.block(C, A).await.unwrap();
        s.mute(B, A).await.unwrap();
        s.create_request(A, B).await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        delete_social_rows_for(&mut conn, "socialprobea")
            .await
            .unwrap();
        drop(conn);
        assert!(s.list_friends(C).await.unwrap().is_empty());
        assert!(s.list_blocks(C).await.unwrap().is_empty());
        assert!(s.list_mutes(B).await.unwrap().is_empty());
        assert!(s.list_incoming(B).await.unwrap().is_empty());
        assert_eq!(n.list(A, None, 50).await.unwrap().len(), 0);

        clean(&pool).await;
    }
}
