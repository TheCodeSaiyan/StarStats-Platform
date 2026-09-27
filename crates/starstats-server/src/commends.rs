//! Crew history and crew commends (social phase 5).
//!
//! **Crew history** is the private list of players you flew with in a
//! Looking for Group post. A pair is written, in both directions, when the
//! host accepts a player, and removed if the host later removes them; a
//! player who leaves still flew. Rows are kept for [`CREW_HISTORY_DAYS`].
//!
//! **A commend** is one word from that list, [`CommendKind`], given to a
//! crewmate in the [`COMMEND_WINDOW_HOURS`] after the post ends. Totals are
//! public; who gave which is not, to anyone, the recipient included. Two
//! guards keep the totals honest:
//!
//! - only players who flew together can commend each other, and giving one
//!   needs a verified RSI handle (the routes enforce both);
//! - **one per pair per week**: a giver's commends to one recipient count
//!   once per calendar week (UTC, Monday start) towards the totals,
//!   whatever the kind, so friends opening throwaway posts cannot farm each
//!   other. Every commend is still stored.
//!
//! Commends from an account whose sharing is restricted stop counting while
//! the restriction lasts, as salutes do. Account deletion removes both
//! tables' rows through `social::delete_social_rows_for`, and a block
//! removes them between the pair.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

/// How long crew history is kept.
pub const CREW_HISTORY_DAYS: i64 = 90;
/// How long after a post ends its crew can commend each other.
pub const COMMEND_WINDOW_HOURS: i64 = 48;

#[derive(Debug, thiserror::Error)]
pub enum CommendError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// The closed vocabulary. Stored as TEXT, so adding a kind needs no
/// migration. There is deliberately no free text and nothing negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommendKind {
    GreatPilot,
    GoodComms,
    Reliable,
    GoodTeacher,
}

impl CommendKind {
    pub const ALL: [CommendKind; 4] = [
        Self::GreatPilot,
        Self::GoodComms,
        Self::Reliable,
        Self::GoodTeacher,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GreatPilot => "great_pilot",
            Self::GoodComms => "good_comms",
            Self::Reliable => "reliable",
            Self::GoodTeacher => "good_teacher",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// One kind's public total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CommendTotal {
    pub kind: CommendKind,
    pub count: i64,
}

/// Someone you flew with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CrewMate {
    pub post_id: Uuid,
    pub handle: String,
    pub activity: String,
    pub flew_at: DateTime<Utc>,
}

/// A commend the caller gave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GivenCommend {
    pub post_id: Uuid,
    pub recipient: String,
    pub kind: CommendKind,
}

#[async_trait]
pub trait CommendStore: Send + Sync + 'static {
    /// Record that `a` and `b` flew together on a post, in both directions.
    /// Idempotent.
    async fn record_crew(
        &self,
        post_id: Uuid,
        activity: &str,
        a: &str,
        b: &str,
        now: DateTime<Utc>,
    ) -> Result<(), CommendError>;
    /// Take a player out of a post's crew (the host removed them), with
    /// any commends given to or by them on it.
    async fn forget_crew(&self, post_id: Uuid, handle: &str) -> Result<u64, CommendError>;
    /// Everyone `handle` flew with since `since`, newest first, one row per
    /// post and crewmate.
    async fn crew_of(
        &self,
        handle: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<CrewMate>, CommendError>;
    async fn flew_together(&self, post_id: Uuid, a: &str, b: &str) -> Result<bool, CommendError>;
    /// Give or change a commend. `true` when it is new.
    async fn set(
        &self,
        post_id: Uuid,
        giver: &str,
        recipient: &str,
        kind: CommendKind,
        now: DateTime<Utc>,
    ) -> Result<bool, CommendError>;
    /// `true` when a commend existed and was removed.
    async fn withdraw(
        &self,
        post_id: Uuid,
        giver: &str,
        recipient: &str,
    ) -> Result<bool, CommendError>;
    /// What `giver` gave on these posts.
    async fn given(
        &self,
        giver: &str,
        post_ids: &[Uuid],
    ) -> Result<Vec<GivenCommend>, CommendError>;
    /// Public totals for every kind, zeros included, under the one per
    /// pair per week rule and leaving out restricted givers.
    async fn totals(&self, recipient: &str) -> Result<Vec<CommendTotal>, CommendError>;
    /// Remove commends and crew history between two users in either
    /// direction (a block).
    async fn delete_between(&self, a: &str, b: &str) -> Result<u64, CommendError>;
    /// Delete crew history older than `before`. Commends stay: they are
    /// the totals.
    async fn purge_crew_before(&self, before: DateTime<Utc>) -> Result<u64, CommendError>;
}

/// Zero-filled totals in [`CommendKind::ALL`] order.
fn fill_totals(counts: impl IntoIterator<Item = (CommendKind, i64)>) -> Vec<CommendTotal> {
    let counts: Vec<(CommendKind, i64)> = counts.into_iter().collect();
    CommendKind::ALL
        .into_iter()
        .map(|kind| CommendTotal {
            kind,
            count: counts
                .iter()
                .filter(|(k, _)| *k == kind)
                .map(|(_, n)| n)
                .sum(),
        })
        .collect()
}

pub struct PostgresCommendStore {
    pool: PgPool,
}

impl PostgresCommendStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CommendStore for PostgresCommendStore {
    async fn record_crew(
        &self,
        post_id: Uuid,
        activity: &str,
        a: &str,
        b: &str,
        now: DateTime<Utc>,
    ) -> Result<(), CommendError> {
        sqlx::query(
            r#"
            INSERT INTO crew_history (post_id, handle, other_handle, activity, flew_at)
            VALUES ($1, $2, $3, $4, $5), ($1, $3, $2, $4, $5)
            ON CONFLICT (post_id, lower(handle), lower(other_handle)) DO NOTHING
            "#,
        )
        .bind(post_id)
        .bind(a)
        .bind(b)
        .bind(activity)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn forget_crew(&self, post_id: Uuid, handle: &str) -> Result<u64, CommendError> {
        let mut tx = self.pool.begin().await?;
        let crew = sqlx::query(
            "DELETE FROM crew_history
             WHERE post_id = $1
               AND (lower(handle) = lower($2) OR lower(other_handle) = lower($2))",
        )
        .bind(post_id)
        .bind(handle)
        .execute(&mut *tx)
        .await?;
        let commends = sqlx::query(
            "DELETE FROM crew_commends
             WHERE post_id = $1
               AND (lower(giver_handle) = lower($2) OR lower(recipient_handle) = lower($2))",
        )
        .bind(post_id)
        .bind(handle)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(crew.rows_affected() + commends.rows_affected())
    }

    async fn crew_of(
        &self,
        handle: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<CrewMate>, CommendError> {
        let rows: Vec<(Uuid, String, String, DateTime<Utc>)> = sqlx::query_as(
            r#"
            SELECT post_id, other_handle, activity, flew_at
            FROM crew_history
            WHERE lower(handle) = lower($1) AND flew_at >= $2
            ORDER BY flew_at DESC, lower(other_handle)
            "#,
        )
        .bind(handle)
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(post_id, handle, activity, flew_at)| CrewMate {
                post_id,
                handle,
                activity,
                flew_at,
            })
            .collect())
    }

    async fn flew_together(&self, post_id: Uuid, a: &str, b: &str) -> Result<bool, CommendError> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM crew_history
             WHERE post_id = $1 AND lower(handle) = lower($2) AND lower(other_handle) = lower($3)",
        )
        .bind(post_id)
        .bind(a)
        .bind(b)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    async fn set(
        &self,
        post_id: Uuid,
        giver: &str,
        recipient: &str,
        kind: CommendKind,
        now: DateTime<Utc>,
    ) -> Result<bool, CommendError> {
        // xmax = 0 only on a freshly inserted row, which tells a new
        // commend from a changed one without a second query.
        let (inserted,): (bool,) = sqlx::query_as(
            r#"
            INSERT INTO crew_commends
                (post_id, giver_handle, recipient_handle, kind, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $5)
            ON CONFLICT (post_id, lower(giver_handle), lower(recipient_handle))
            DO UPDATE SET kind = EXCLUDED.kind, updated_at = EXCLUDED.updated_at
            RETURNING (xmax = 0)
            "#,
        )
        .bind(post_id)
        .bind(giver)
        .bind(recipient)
        .bind(kind.as_str())
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(inserted)
    }

    async fn withdraw(
        &self,
        post_id: Uuid,
        giver: &str,
        recipient: &str,
    ) -> Result<bool, CommendError> {
        let res = sqlx::query(
            "DELETE FROM crew_commends
             WHERE post_id = $1
               AND lower(giver_handle) = lower($2) AND lower(recipient_handle) = lower($3)",
        )
        .bind(post_id)
        .bind(giver)
        .bind(recipient)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn given(
        &self,
        giver: &str,
        post_ids: &[Uuid],
    ) -> Result<Vec<GivenCommend>, CommendError> {
        if post_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
            "SELECT post_id, recipient_handle, kind FROM crew_commends
             WHERE lower(giver_handle) = lower($1) AND post_id = ANY($2)",
        )
        .bind(giver)
        .bind(post_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(post_id, recipient, kind)| {
                Some(GivenCommend {
                    post_id,
                    recipient,
                    kind: CommendKind::parse(&kind)?,
                })
            })
            .collect())
    }

    async fn totals(&self, recipient: &str) -> Result<Vec<CommendTotal>, CommendError> {
        // The earliest commend per giver per UTC week is the one that
        // counts; the rest are stored but not totalled.
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT kind, count(*)
            FROM (
                SELECT DISTINCT ON (
                    lower(c.giver_handle),
                    date_trunc('week', c.created_at AT TIME ZONE 'UTC')
                ) c.kind
                FROM crew_commends c
                WHERE lower(c.recipient_handle) = lower($1)
                  AND NOT EXISTS (
                      SELECT 1
                      FROM account_restrictions r
                      JOIN users u ON u.id = r.user_id
                      WHERE lower(u.claimed_handle) = lower(c.giver_handle)
                        AND r.sharing_blocked
                        AND (r.expires_at IS NULL OR r.expires_at > now())
                  )
                ORDER BY
                    lower(c.giver_handle),
                    date_trunc('week', c.created_at AT TIME ZONE 'UTC'),
                    c.created_at
            ) counted
            GROUP BY kind
            "#,
        )
        .bind(recipient)
        .fetch_all(&self.pool)
        .await?;
        Ok(fill_totals(rows.into_iter().filter_map(|(k, n)| {
            CommendKind::parse(&k).map(|k| (k, n))
        })))
    }

    async fn delete_between(&self, a: &str, b: &str) -> Result<u64, CommendError> {
        let mut tx = self.pool.begin().await?;
        let crew = sqlx::query(
            "DELETE FROM crew_history
             WHERE (lower(handle) = lower($1) AND lower(other_handle) = lower($2))
                OR (lower(handle) = lower($2) AND lower(other_handle) = lower($1))",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await?;
        let commends = sqlx::query(
            "DELETE FROM crew_commends
             WHERE (lower(giver_handle) = lower($1) AND lower(recipient_handle) = lower($2))
                OR (lower(giver_handle) = lower($2) AND lower(recipient_handle) = lower($1))",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(crew.rows_affected() + commends.rows_affected())
    }

    async fn purge_crew_before(&self, before: DateTime<Utc>) -> Result<u64, CommendError> {
        let res = sqlx::query("DELETE FROM crew_history WHERE flew_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }
}

/// Per-user token bucket for giving and withdrawing commends, with the
/// salute limiter's budget: each commend notifies someone, so a loop of
/// give and withdraw would otherwise flood them. Its own type so it is a
/// separate Extension from the salute limiter.
#[derive(Default)]
pub struct CommendRateLimiter(crate::salutes::SaluteRateLimiter);

impl CommendRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when the action may go ahead.
    pub fn check(&self, handle: &str) -> bool {
        self.0.check(handle)
    }
}

#[cfg(test)]
/// The UTC week a commend falls in, for the one per pair per week rule.
/// Matches `date_trunc('week', ts AT TIME ZONE 'UTC')`: ISO weeks start on
/// Monday.
fn week_of(t: DateTime<Utc>) -> (i32, u32) {
    use chrono::Datelike;
    let w = t.iso_week();
    (w.year(), w.week())
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    fn eq(a: &str, b: &str) -> bool {
        a.eq_ignore_ascii_case(b)
    }

    #[derive(Clone)]
    struct Commend {
        post_id: Uuid,
        giver: String,
        recipient: String,
        kind: CommendKind,
        created_at: DateTime<Utc>,
    }

    #[derive(Default)]
    pub struct MemoryCommendStore {
        crew: Mutex<Vec<CrewMateRow>>,
        commends: Mutex<Vec<Commend>>,
        restricted: Mutex<HashSet<String>>,
    }

    #[derive(Clone)]
    struct CrewMateRow {
        handle: String,
        mate: CrewMate,
    }

    impl MemoryCommendStore {
        pub fn new() -> Self {
            Self::default()
        }

        /// Stand-in for an account whose sharing is restricted.
        pub fn restrict(&self, handle: &str) {
            self.restricted
                .lock()
                .unwrap()
                .insert(handle.to_lowercase());
        }
    }

    #[async_trait]
    impl CommendStore for MemoryCommendStore {
        async fn record_crew(
            &self,
            post_id: Uuid,
            activity: &str,
            a: &str,
            b: &str,
            now: DateTime<Utc>,
        ) -> Result<(), CommendError> {
            let mut crew = self.crew.lock().unwrap();
            for (me, other) in [(a, b), (b, a)] {
                if crew.iter().any(|r| {
                    r.mate.post_id == post_id && eq(&r.handle, me) && eq(&r.mate.handle, other)
                }) {
                    continue;
                }
                crew.push(CrewMateRow {
                    handle: me.into(),
                    mate: CrewMate {
                        post_id,
                        handle: other.into(),
                        activity: activity.into(),
                        flew_at: now,
                    },
                });
            }
            Ok(())
        }

        async fn forget_crew(&self, post_id: Uuid, handle: &str) -> Result<u64, CommendError> {
            let mut crew = self.crew.lock().unwrap();
            let mut commends = self.commends.lock().unwrap();
            let before = crew.len() + commends.len();
            crew.retain(|r| {
                !(r.mate.post_id == post_id
                    && (eq(&r.handle, handle) || eq(&r.mate.handle, handle)))
            });
            commends.retain(|c| {
                !(c.post_id == post_id && (eq(&c.giver, handle) || eq(&c.recipient, handle)))
            });
            Ok((before - crew.len() - commends.len()) as u64)
        }

        async fn crew_of(
            &self,
            handle: &str,
            since: DateTime<Utc>,
        ) -> Result<Vec<CrewMate>, CommendError> {
            let mut v: Vec<CrewMate> = self
                .crew
                .lock()
                .unwrap()
                .iter()
                .filter(|r| eq(&r.handle, handle) && r.mate.flew_at >= since)
                .map(|r| r.mate.clone())
                .collect();
            v.sort_by(|a, b| {
                b.flew_at
                    .cmp(&a.flew_at)
                    .then_with(|| a.handle.to_lowercase().cmp(&b.handle.to_lowercase()))
            });
            Ok(v)
        }

        async fn flew_together(
            &self,
            post_id: Uuid,
            a: &str,
            b: &str,
        ) -> Result<bool, CommendError> {
            Ok(self
                .crew
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.mate.post_id == post_id && eq(&r.handle, a) && eq(&r.mate.handle, b)))
        }

        async fn set(
            &self,
            post_id: Uuid,
            giver: &str,
            recipient: &str,
            kind: CommendKind,
            now: DateTime<Utc>,
        ) -> Result<bool, CommendError> {
            let mut commends = self.commends.lock().unwrap();
            if let Some(c) = commends.iter_mut().find(|c| {
                c.post_id == post_id && eq(&c.giver, giver) && eq(&c.recipient, recipient)
            }) {
                c.kind = kind;
                return Ok(false);
            }
            commends.push(Commend {
                post_id,
                giver: giver.into(),
                recipient: recipient.into(),
                kind,
                created_at: now,
            });
            Ok(true)
        }

        async fn withdraw(
            &self,
            post_id: Uuid,
            giver: &str,
            recipient: &str,
        ) -> Result<bool, CommendError> {
            let mut commends = self.commends.lock().unwrap();
            let before = commends.len();
            commends.retain(|c| {
                !(c.post_id == post_id && eq(&c.giver, giver) && eq(&c.recipient, recipient))
            });
            Ok(commends.len() < before)
        }

        async fn given(
            &self,
            giver: &str,
            post_ids: &[Uuid],
        ) -> Result<Vec<GivenCommend>, CommendError> {
            Ok(self
                .commends
                .lock()
                .unwrap()
                .iter()
                .filter(|c| eq(&c.giver, giver) && post_ids.contains(&c.post_id))
                .map(|c| GivenCommend {
                    post_id: c.post_id,
                    recipient: c.recipient.clone(),
                    kind: c.kind,
                })
                .collect())
        }

        async fn totals(&self, recipient: &str) -> Result<Vec<CommendTotal>, CommendError> {
            let restricted = self.restricted.lock().unwrap();
            let mut mine: Vec<Commend> = self
                .commends
                .lock()
                .unwrap()
                .iter()
                .filter(|c| eq(&c.recipient, recipient))
                .filter(|c| !restricted.contains(&c.giver.to_lowercase()))
                .cloned()
                .collect();
            mine.sort_by_key(|c| c.created_at);
            let mut seen: HashSet<(String, (i32, u32))> = HashSet::new();
            let counted = mine
                .into_iter()
                .filter(|c| seen.insert((c.giver.to_lowercase(), week_of(c.created_at))))
                .map(|c| (c.kind, 1));
            Ok(fill_totals(counted))
        }

        async fn delete_between(&self, a: &str, b: &str) -> Result<u64, CommendError> {
            let pair = |x: &str, y: &str| (eq(x, a) && eq(y, b)) || (eq(x, b) && eq(y, a));
            let mut crew = self.crew.lock().unwrap();
            let mut commends = self.commends.lock().unwrap();
            let before = crew.len() + commends.len();
            crew.retain(|r| !pair(&r.handle, &r.mate.handle));
            commends.retain(|c| !pair(&c.giver, &c.recipient));
            Ok((before - crew.len() - commends.len()) as u64)
        }

        async fn purge_crew_before(&self, before: DateTime<Utc>) -> Result<u64, CommendError> {
            let mut crew = self.crew.lock().unwrap();
            let n = crew.len();
            crew.retain(|r| r.mate.flew_at >= before);
            Ok((n - crew.len()) as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryCommendStore;
    use super::*;
    use chrono::Duration;

    fn at(day: i64) -> DateTime<Utc> {
        // 2026-09-28 is a Monday, so days 0..=6 are one ISO week.
        DateTime::parse_from_rfc3339("2026-09-28T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::days(day)
    }

    fn total(t: &[CommendTotal], kind: CommendKind) -> i64 {
        t.iter().find(|x| x.kind == kind).unwrap().count
    }

    #[test]
    fn kinds_round_trip_and_unknown_ones_are_refused() {
        for k in CommendKind::ALL {
            assert_eq!(CommendKind::parse(k.as_str()), Some(k));
            assert_eq!(
                serde_json::to_value(k).unwrap(),
                serde_json::json!(k.as_str())
            );
        }
        assert_eq!(CommendKind::parse("toxic"), None);
    }

    #[test]
    fn weeks_start_on_monday_utc() {
        assert_eq!(week_of(at(0)), week_of(at(6)), "Monday to Sunday");
        assert_ne!(week_of(at(6)), week_of(at(7)), "the next Monday");
    }

    #[tokio::test]
    async fn crew_is_recorded_both_ways_once() {
        let s = MemoryCommendStore::new();
        let p = Uuid::new_v4();
        s.record_crew(p, "mining", "Host", "Bob", at(0))
            .await
            .unwrap();
        s.record_crew(p, "mining", "host", "BOB", at(0))
            .await
            .unwrap();
        assert!(s.flew_together(p, "bob", "HOST").await.unwrap());
        assert!(s.flew_together(p, "Host", "Bob").await.unwrap());
        assert!(!s
            .flew_together(Uuid::new_v4(), "Host", "Bob")
            .await
            .unwrap());
        let crew = s.crew_of("host", at(-1)).await.unwrap();
        assert_eq!(crew.len(), 1);
        assert_eq!(crew[0].handle, "Bob");
    }

    #[tokio::test]
    async fn a_new_commend_is_new_and_a_changed_one_is_not() {
        let s = MemoryCommendStore::new();
        let p = Uuid::new_v4();
        assert!(s
            .set(p, "Alice", "Bob", CommendKind::GreatPilot, at(0))
            .await
            .unwrap());
        assert!(!s
            .set(p, "alice", "BOB", CommendKind::GoodComms, at(0))
            .await
            .unwrap());
        let t = s.totals("bob").await.unwrap();
        assert_eq!(total(&t, CommendKind::GoodComms), 1, "the change stuck");
        assert_eq!(total(&t, CommendKind::GreatPilot), 0);
        assert_eq!(t.len(), CommendKind::ALL.len(), "zeros are listed");
        assert!(s.withdraw(p, "ALICE", "bob").await.unwrap());
        assert!(!s.withdraw(p, "ALICE", "bob").await.unwrap());
    }

    #[tokio::test]
    async fn one_commend_per_pair_per_week_counts() {
        let s = MemoryCommendStore::new();
        // Alice farms Bob with a fresh post every day for two weeks.
        for day in 0..14 {
            s.set(
                Uuid::new_v4(),
                "Alice",
                "Bob",
                CommendKind::GreatPilot,
                at(day),
            )
            .await
            .unwrap();
        }
        s.set(Uuid::new_v4(), "Carol", "Bob", CommendKind::Reliable, at(1))
            .await
            .unwrap();
        let t = s.totals("Bob").await.unwrap();
        assert_eq!(total(&t, CommendKind::GreatPilot), 2, "two weeks, two");
        assert_eq!(total(&t, CommendKind::Reliable), 1, "another giver counts");
    }

    #[tokio::test]
    async fn a_restricted_giver_stops_counting() {
        let s = MemoryCommendStore::new();
        s.set(
            Uuid::new_v4(),
            "Mallory",
            "Bob",
            CommendKind::Reliable,
            at(0),
        )
        .await
        .unwrap();
        s.restrict("mallory");
        assert_eq!(
            total(&s.totals("Bob").await.unwrap(), CommendKind::Reliable),
            0
        );
    }

    #[tokio::test]
    async fn forgetting_a_crewmate_takes_their_commends_on_that_post() {
        let s = MemoryCommendStore::new();
        let (p, other) = (Uuid::new_v4(), Uuid::new_v4());
        s.record_crew(p, "mining", "Host", "Bob", at(0))
            .await
            .unwrap();
        s.record_crew(p, "mining", "Host", "Carol", at(0))
            .await
            .unwrap();
        s.set(p, "Bob", "Host", CommendKind::Reliable, at(0))
            .await
            .unwrap();
        s.set(p, "Carol", "Host", CommendKind::Reliable, at(0))
            .await
            .unwrap();
        s.set(other, "Bob", "Host", CommendKind::GoodComms, at(8))
            .await
            .unwrap();
        s.forget_crew(p, "bob").await.unwrap();
        assert!(!s.flew_together(p, "Host", "Bob").await.unwrap());
        assert!(s.flew_together(p, "Host", "Carol").await.unwrap());
        let t = s.totals("Host").await.unwrap();
        assert_eq!(total(&t, CommendKind::Reliable), 1, "Carol's stays");
        assert_eq!(total(&t, CommendKind::GoodComms), 1, "another post's stays");
    }

    #[tokio::test]
    async fn a_block_removes_both_directions_and_nothing_else() {
        let s = MemoryCommendStore::new();
        let p = Uuid::new_v4();
        s.record_crew(p, "mining", "Alice", "Bob", at(0))
            .await
            .unwrap();
        s.record_crew(p, "mining", "Alice", "Carol", at(0))
            .await
            .unwrap();
        s.set(p, "Alice", "Bob", CommendKind::Reliable, at(0))
            .await
            .unwrap();
        s.set(p, "Bob", "Alice", CommendKind::Reliable, at(0))
            .await
            .unwrap();
        s.set(p, "Carol", "Alice", CommendKind::Reliable, at(0))
            .await
            .unwrap();
        s.delete_between("BOB", "alice").await.unwrap();
        assert!(!s.flew_together(p, "Alice", "Bob").await.unwrap());
        assert!(s.flew_together(p, "Alice", "Carol").await.unwrap());
        assert_eq!(
            total(&s.totals("Alice").await.unwrap(), CommendKind::Reliable),
            1
        );
        assert_eq!(
            total(&s.totals("Bob").await.unwrap(), CommendKind::Reliable),
            0
        );
    }

    #[tokio::test]
    async fn purging_old_crew_history_keeps_the_totals() {
        let s = MemoryCommendStore::new();
        let p = Uuid::new_v4();
        s.record_crew(p, "mining", "Alice", "Bob", at(0))
            .await
            .unwrap();
        s.set(p, "Alice", "Bob", CommendKind::Reliable, at(1))
            .await
            .unwrap();
        assert_eq!(s.purge_crew_before(at(91)).await.unwrap(), 2);
        assert!(s.crew_of("Alice", at(-1)).await.unwrap().is_empty());
        assert_eq!(
            total(&s.totals("Bob").await.unwrap(), CommendKind::Reliable),
            1
        );
    }

    /// The same behaviour against the real schema, including the week
    /// rule's SQL. Skipped without STARSTATS_TEST_DATABASE_URL.
    #[tokio::test]
    async fn postgres_commends_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres commends test");
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
        let s = PostgresCommendStore::new(pool);
        // Unique per run, so reruns against one database do not collide.
        let tag = &Uuid::new_v4().simple().to_string()[..8];
        let (host, bob, carol) = (
            format!("cmHost{tag}"),
            format!("cmBob{tag}"),
            format!("cmCarol{tag}"),
        );
        let p = Uuid::new_v4();

        s.record_crew(p, "mining", &host, &bob, at(0))
            .await
            .unwrap();
        s.record_crew(p, "mining", &host.to_uppercase(), &bob, at(0))
            .await
            .unwrap();
        s.record_crew(p, "mining", &host, &carol, at(0))
            .await
            .unwrap();
        assert!(s
            .flew_together(p, &bob.to_lowercase(), &host)
            .await
            .unwrap());
        assert_eq!(s.crew_of(&host, at(-1)).await.unwrap().len(), 2);

        assert!(s
            .set(p, &bob, &host, CommendKind::GreatPilot, at(0))
            .await
            .unwrap());
        assert!(!s
            .set(p, &bob, &host, CommendKind::GoodComms, at(0))
            .await
            .unwrap());
        for day in 1..10 {
            s.set(
                Uuid::new_v4(),
                &bob,
                &host,
                CommendKind::GreatPilot,
                at(day),
            )
            .await
            .unwrap();
        }
        s.set(p, &carol, &host, CommendKind::Reliable, at(2))
            .await
            .unwrap();
        let t = s.totals(&host).await.unwrap();
        assert_eq!(
            total(&t, CommendKind::GoodComms) + total(&t, CommendKind::GreatPilot),
            2,
            "Bob counts once in each of two weeks"
        );
        assert_eq!(
            total(&t, CommendKind::GoodComms),
            1,
            "the first of week one"
        );
        assert_eq!(total(&t, CommendKind::Reliable), 1);

        let given = s.given(&bob, &[p]).await.unwrap();
        assert_eq!(given.len(), 1);
        assert_eq!(given[0].kind, CommendKind::GoodComms);

        s.forget_crew(p, &carol).await.unwrap();
        assert_eq!(
            total(&s.totals(&host).await.unwrap(), CommendKind::Reliable),
            0
        );
        s.delete_between(&host, &bob).await.unwrap();
        assert!(!s.flew_together(p, &host, &bob).await.unwrap());
        assert_eq!(
            s.totals(&host)
                .await
                .unwrap()
                .iter()
                .map(|t| t.count)
                .sum::<i64>(),
            0
        );
        assert!(s.withdraw(p, &bob, &host).await.is_ok());
    }
}
