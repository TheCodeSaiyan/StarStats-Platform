//! Session projection feeding the pure fact engine.
//!
//! [`crate::facts`] is deliberately store-free, so the one query it needs
//! lives here. Reads the materialized `session_summary` rollup rather than
//! walking `events`: the whole catalogue is served by a single bounded read.

use crate::facts::SessionFacts;
use crate::repo::RepoError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Most sessions considered. A prolific player has thousands; the rules are
/// all ratios and medians, so the newest slice describes them just as well
/// and the read stays bounded. Ordered newest-first, so the trailing-window
/// rules (pace, tempo, cadence) always see their full window.
pub const MAX_SESSIONS: i64 = 2_000;

#[async_trait]
pub trait FactsStore: Send + Sync + 'static {
    async fn sessions_for_facts(&self, handle: &str) -> Result<Vec<SessionFacts>, RepoError>;
}

pub struct PostgresFactsStore {
    pool: PgPool,
}

impl PostgresFactsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl FactsStore for PostgresFactsStore {
    async fn sessions_for_facts(&self, handle: &str) -> Result<Vec<SessionFacts>, RepoError> {
        // The rollup is rebuilt lazily. Without this a cold or dirty handle
        // reads an empty table and every fact silently disappears — the
        // widget would show "not enough flight time yet" to a veteran.
        crate::repo::PostgresStore::new(self.pool.clone())
            .ensure_session_stats_fresh(handle)
            .await?;

        let rows: Vec<(DateTime<Utc>, DateTime<Utc>, i64)> = sqlx::query_as(
            "SELECT started_at, ended_at, death_count
             FROM session_summary
             WHERE claimed_handle = LOWER($1)
               AND started_at IS NOT NULL AND ended_at IS NOT NULL
             ORDER BY started_at DESC
             LIMIT $2",
        )
        .bind(handle)
        .bind(MAX_SESSIONS)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(started_at, ended_at, death_count)| SessionFacts {
                started_at,
                ended_at,
                death_count,
            })
            .collect())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryFactsStore {
        sessions: Mutex<Vec<SessionFacts>>,
    }

    impl MemoryFactsStore {
        pub fn new(sessions: Vec<SessionFacts>) -> Self {
            Self {
                sessions: Mutex::new(sessions),
            }
        }
    }

    #[async_trait]
    impl FactsStore for MemoryFactsStore {
        async fn sessions_for_facts(&self, _handle: &str) -> Result<Vec<SessionFacts>, RepoError> {
            Ok(self.sessions.lock().unwrap().clone())
        }
    }
}
