//! o7 salutes: a public per-profile accolade (social phase 2).
//!
//! One salute per viewer per profile. The count is public; who saluted
//! is not, except that an owner sees which of their friends did. Because
//! the count is public it will attract sock puppets, so the routes
//! require a verified RSI handle, and a salute from an account whose
//! sharing is restricted stops counting while the restriction lasts
//! (the row stays, so lifting the restriction restores it).
//!
//! Rows are keyed on handles, case-insensitively, like friendships and
//! blocks, and account deletion removes them through
//! `social::delete_social_rows_for`.

use async_trait::async_trait;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum SaluteError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait SaluteStore: Send + Sync + 'static {
    /// Record a salute. `true` when it is new, `false` when the viewer
    /// had already saluted this profile.
    async fn salute(&self, saluter: &str, target: &str) -> Result<bool, SaluteError>;
    /// `true` when a salute existed and was removed.
    async fn unsalute(&self, saluter: &str, target: &str) -> Result<bool, SaluteError>;
    async fn has_saluted(&self, saluter: &str, target: &str) -> Result<bool, SaluteError>;
    /// The public count, leaving out saluters whose sharing is
    /// currently restricted.
    async fn count(&self, target: &str) -> Result<i64, SaluteError>;
    /// Which of `candidates` have saluted `target`, in the candidates'
    /// own spelling. For "friends who saluted", so never a full list.
    async fn saluters_among(
        &self,
        target: &str,
        candidates: &[String],
    ) -> Result<Vec<String>, SaluteError>;
    /// Remove salutes between two users in either direction (a block).
    async fn delete_between(&self, a: &str, b: &str) -> Result<u64, SaluteError>;
}

pub struct PostgresSaluteStore {
    pool: PgPool,
}

impl PostgresSaluteStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SaluteStore for PostgresSaluteStore {
    async fn salute(&self, saluter: &str, target: &str) -> Result<bool, SaluteError> {
        let res = sqlx::query(
            r#"
            INSERT INTO profile_salutes (saluter_handle, target_handle) VALUES ($1, $2)
            ON CONFLICT (lower(saluter_handle), lower(target_handle)) DO NOTHING
            "#,
        )
        .bind(saluter)
        .bind(target)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn unsalute(&self, saluter: &str, target: &str) -> Result<bool, SaluteError> {
        let res = sqlx::query(
            "DELETE FROM profile_salutes
             WHERE lower(saluter_handle) = lower($1) AND lower(target_handle) = lower($2)",
        )
        .bind(saluter)
        .bind(target)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn has_saluted(&self, saluter: &str, target: &str) -> Result<bool, SaluteError> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM profile_salutes
             WHERE lower(saluter_handle) = lower($1) AND lower(target_handle) = lower($2)",
        )
        .bind(saluter)
        .bind(target)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    async fn count(&self, target: &str) -> Result<i64, SaluteError> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT count(*)
            FROM profile_salutes s
            WHERE lower(s.target_handle) = lower($1)
              AND NOT EXISTS (
                  SELECT 1
                  FROM account_restrictions r
                  JOIN users u ON u.id = r.user_id
                  WHERE lower(u.claimed_handle) = lower(s.saluter_handle)
                    AND r.sharing_blocked
                    AND (r.expires_at IS NULL OR r.expires_at > now())
              )
            "#,
        )
        .bind(target)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    async fn saluters_among(
        &self,
        target: &str,
        candidates: &[String],
    ) -> Result<Vec<String>, SaluteError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let lowered: Vec<String> = candidates.iter().map(|c| c.to_lowercase()).collect();
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT lower(saluter_handle) FROM profile_salutes
             WHERE lower(target_handle) = lower($1) AND lower(saluter_handle) = ANY($2)",
        )
        .bind(target)
        .bind(&lowered)
        .fetch_all(&self.pool)
        .await?;
        let hit: std::collections::HashSet<String> = rows.into_iter().map(|(h,)| h).collect();
        Ok(candidates
            .iter()
            .filter(|c| hit.contains(&c.to_lowercase()))
            .cloned()
            .collect())
    }

    async fn delete_between(&self, a: &str, b: &str) -> Result<u64, SaluteError> {
        let res = sqlx::query(
            "DELETE FROM profile_salutes
             WHERE (lower(saluter_handle) = lower($1) AND lower(target_handle) = lower($2))
                OR (lower(saluter_handle) = lower($2) AND lower(target_handle) = lower($1))",
        )
        .bind(a)
        .bind(b)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }
}

/// Burst of salutes a user can make before the limiter bites, and how
/// fast it refills. Unsaluting and saluting again notifies the owner
/// again, so without this a script could flood someone's inbox.
pub const SALUTE_BURST: f64 = 10.0;
pub const SALUTE_REFILL_PER_SEC: f64 = 1.0 / 30.0;

/// Per-user token bucket for salute and unsalute, keyed on the
/// lower-cased handle. In-process, like the roadmap vote limiter.
#[derive(Default)]
pub struct SaluteRateLimiter {
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl SaluteRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when the action may go ahead.
    pub fn check(&self, handle: &str) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let now = Instant::now();
        let (tokens, last) = buckets
            .entry(handle.to_lowercase())
            .or_insert((SALUTE_BURST, now));
        *tokens = (*tokens + now.duration_since(*last).as_secs_f64() * SALUTE_REFILL_PER_SEC)
            .min(SALUTE_BURST);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashSet;

    fn eq(a: &str, b: &str) -> bool {
        a.eq_ignore_ascii_case(b)
    }

    #[derive(Default)]
    pub struct MemorySaluteStore {
        rows: Mutex<Vec<(String, String)>>,
        restricted: Mutex<HashSet<String>>,
    }

    impl MemorySaluteStore {
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
    impl SaluteStore for MemorySaluteStore {
        async fn salute(&self, saluter: &str, target: &str) -> Result<bool, SaluteError> {
            let mut rows = self.rows.lock().unwrap();
            if rows.iter().any(|(s, t)| eq(s, saluter) && eq(t, target)) {
                return Ok(false);
            }
            rows.push((saluter.into(), target.into()));
            Ok(true)
        }

        async fn unsalute(&self, saluter: &str, target: &str) -> Result<bool, SaluteError> {
            let mut rows = self.rows.lock().unwrap();
            let before = rows.len();
            rows.retain(|(s, t)| !(eq(s, saluter) && eq(t, target)));
            Ok(rows.len() < before)
        }

        async fn has_saluted(&self, saluter: &str, target: &str) -> Result<bool, SaluteError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows.iter().any(|(s, t)| eq(s, saluter) && eq(t, target)))
        }

        async fn count(&self, target: &str) -> Result<i64, SaluteError> {
            let rows = self.rows.lock().unwrap();
            let restricted = self.restricted.lock().unwrap();
            Ok(rows
                .iter()
                .filter(|(s, t)| eq(t, target) && !restricted.contains(&s.to_lowercase()))
                .count() as i64)
        }

        async fn saluters_among(
            &self,
            target: &str,
            candidates: &[String],
        ) -> Result<Vec<String>, SaluteError> {
            let rows = self.rows.lock().unwrap();
            Ok(candidates
                .iter()
                .filter(|c| rows.iter().any(|(s, t)| eq(s, c) && eq(t, target)))
                .cloned()
                .collect())
        }

        async fn delete_between(&self, a: &str, b: &str) -> Result<u64, SaluteError> {
            let mut rows = self.rows.lock().unwrap();
            let before = rows.len();
            rows.retain(|(s, t)| !((eq(s, a) && eq(t, b)) || (eq(s, b) && eq(t, a))));
            Ok((before - rows.len()) as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemorySaluteStore;
    use super::*;

    #[tokio::test]
    async fn one_salute_per_viewer_whatever_the_case() {
        let s = MemorySaluteStore::new();
        assert!(s.salute("Alice", "Bob").await.unwrap());
        assert!(!s.salute("ALICE", "bob").await.unwrap());
        assert_eq!(s.count("BOB").await.unwrap(), 1);
        assert!(s.has_saluted("alice", "Bob").await.unwrap());
    }

    #[tokio::test]
    async fn unsalute_removes_only_that_salute() {
        let s = MemorySaluteStore::new();
        s.salute("Alice", "Bob").await.unwrap();
        s.salute("Carol", "Bob").await.unwrap();
        assert!(s.unsalute("alice", "BOB").await.unwrap());
        assert!(!s.unsalute("alice", "BOB").await.unwrap());
        assert_eq!(s.count("Bob").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_restricted_saluter_stops_counting() {
        let s = MemorySaluteStore::new();
        s.salute("Alice", "Bob").await.unwrap();
        s.salute("Mallory", "Bob").await.unwrap();
        s.restrict("mallory");
        assert_eq!(s.count("Bob").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn saluters_among_keeps_the_candidates_spelling() {
        let s = MemorySaluteStore::new();
        s.salute("alice", "Bob").await.unwrap();
        s.salute("Stranger", "Bob").await.unwrap();
        let among = s
            .saluters_among("Bob", &["Alice".into(), "Carol".into()])
            .await
            .unwrap();
        assert_eq!(among, vec!["Alice".to_string()]);
    }

    #[tokio::test]
    async fn delete_between_goes_both_ways_and_nowhere_else() {
        let s = MemorySaluteStore::new();
        s.salute("Alice", "Bob").await.unwrap();
        s.salute("Bob", "Alice").await.unwrap();
        s.salute("Carol", "Bob").await.unwrap();
        assert_eq!(s.delete_between("bob", "ALICE").await.unwrap(), 2);
        assert_eq!(s.count("Bob").await.unwrap(), 1);
    }

    #[test]
    fn the_limiter_allows_a_burst_then_refuses() {
        let l = SaluteRateLimiter::new();
        for _ in 0..SALUTE_BURST as usize {
            assert!(l.check("Alice"));
        }
        assert!(!l.check("ALICE"), "the bucket is shared across spellings");
        assert!(l.check("Bob"), "and is per user");
    }

    /// The restriction filter is SQL, so it is only proved against a
    /// real database. Skipped without STARSTATS_TEST_DATABASE_URL.
    #[tokio::test]
    async fn postgres_salutes_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres salutes test");
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
        const A: &str = "SaluteProbeA";
        const B: &str = "saluteprobeb";
        const M: &str = "SaluteProbeM";
        let clean = || async {
            for h in [A, B, M] {
                sqlx::query("DELETE FROM profile_salutes WHERE lower(saluter_handle) = lower($1) OR lower(target_handle) = lower($1)")
                    .bind(h)
                    .execute(&pool)
                    .await
                    .unwrap();
                sqlx::query("DELETE FROM account_restrictions WHERE user_id IN (SELECT id FROM users WHERE lower(claimed_handle) = lower($1))")
                    .bind(h)
                    .execute(&pool)
                    .await
                    .unwrap();
                sqlx::query("DELETE FROM users WHERE lower(claimed_handle) = lower($1)")
                    .bind(h)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        };
        clean().await;
        for h in [A, B, M] {
            sqlx::query(
                "INSERT INTO users (id, email, password_hash, claimed_handle)
                 VALUES (gen_random_uuid(), $1, 'x', $2)",
            )
            .bind(format!("{}@example.com", h.to_ascii_lowercase()))
            .bind(h)
            .execute(&pool)
            .await
            .unwrap();
        }
        let s = PostgresSaluteStore::new(pool.clone());
        assert!(s.salute(A, B).await.unwrap());
        assert!(!s.salute("SALUTEPROBEA", B).await.unwrap());
        assert!(s.salute(M, B).await.unwrap());
        assert_eq!(s.count(B).await.unwrap(), 2);

        // A sharing restriction on M hides M's salute; an expired one does not.
        sqlx::query(
            "INSERT INTO account_restrictions
                 (user_id, sharing_blocked, reason, restricted_by, restricted_at, expires_at)
             SELECT id, true, 'probe', 'probe', now(), now() + interval '1 day'
             FROM users WHERE claimed_handle = $1",
        )
        .bind(M)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(s.count(B).await.unwrap(), 1);
        sqlx::query(
            "UPDATE account_restrictions SET expires_at = now() - interval '1 minute'
             WHERE user_id = (SELECT id FROM users WHERE claimed_handle = $1)",
        )
        .bind(M)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            s.count(B).await.unwrap(),
            2,
            "a lapsed restriction counts again"
        );

        assert_eq!(
            s.saluters_among(B, &["SaluteProbeA".into(), "nobody".into()])
                .await
                .unwrap(),
            vec!["SaluteProbeA".to_string()]
        );
        assert_eq!(s.delete_between(B, A).await.unwrap(), 1);
        assert!(s.unsalute(M, B).await.unwrap());
        assert_eq!(s.count(B).await.unwrap(), 0);
        clean().await;
    }
}
