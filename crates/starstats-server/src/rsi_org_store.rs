//! Persistent store for RSI org-membership snapshots.
//!
//! Mirrors [`crate::hangar_store`] in shape: one row per user, full org
//! list packed into a JSONB column, replaced wholesale on every refresh.
//! Volume is tiny (most users belong to <=20 orgs) — the simpler
//! one-row-per-user shape avoids delete-orphan bookkeeping.
//!
//! Source data is the public `/citizens/{handle}/organizations` page
//! scraped by [`crate::rsi_verify::HttpRsiClient::fetch_orgs`] — no tray
//! cookie, same posture as the citizen profile snapshot.

use crate::rsi_verify::RsiOrg;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// One row of the `rsi_org_snapshots` table. Returned from the store
/// and from the route layer directly — `RsiOrg` carries `ToSchema` so
/// no wire-shape mirror is required.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct RsiOrgsSnapshot {
    pub captured_at: DateTime<Utc>,
    pub orgs: Vec<RsiOrg>,
}

#[derive(Debug, thiserror::Error)]
pub enum RsiOrgStoreError {
    /// Anything the storage backend rejected -- wrapped sqlx error,
    /// JSON encode failure, etc. The route layer surfaces this as
    /// 503; the message lands in the trace span for ops.
    #[error("rsi org store backend error: {0}")]
    Backend(String),
}

impl From<sqlx::Error> for RsiOrgStoreError {
    fn from(err: sqlx::Error) -> Self {
        Self::Backend(err.to_string())
    }
}

#[async_trait]
pub trait RsiOrgStore: Send + Sync + 'static {
    /// Replace the user's full org-membership snapshot. Atomic -- partial
    /// writes are not visible. Returns the just-saved snapshot with
    /// the server-stamped `captured_at`.
    async fn save(
        &self,
        user_id: Uuid,
        orgs: &[RsiOrg],
    ) -> Result<RsiOrgsSnapshot, RsiOrgStoreError>;

    /// Fetch the latest snapshot for a user, or `None` if never
    /// refreshed.
    async fn latest_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Option<RsiOrgsSnapshot>, RsiOrgStoreError>;
}

// -- Postgres impl ---------------------------------------------------

type SnapshotRow = (DateTime<Utc>, sqlx::types::Json<Vec<RsiOrg>>);

pub struct PostgresRsiOrgStore {
    pool: PgPool,
}

impl PostgresRsiOrgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RsiOrgStore for PostgresRsiOrgStore {
    async fn save(
        &self,
        user_id: Uuid,
        orgs: &[RsiOrg],
    ) -> Result<RsiOrgsSnapshot, RsiOrgStoreError> {
        // Bind `orgs` through `sqlx::types::Json` so serde does the
        // JSONB encoding inline -- mirrors `hangar_store::put_snapshot`.
        // ON CONFLICT keeps the upsert atomic; `captured_at` is bumped
        // to NOW() on every push so a "force refresh" from the UI
        // advances the timestamp even if the org list is byte-identical.
        let orgs_json = sqlx::types::Json(orgs);
        let row: SnapshotRow = sqlx::query_as(
            r#"
            INSERT INTO rsi_org_snapshots (user_id, captured_at, orgs)
            VALUES ($1, NOW(), $2)
            ON CONFLICT (user_id) DO UPDATE
                SET captured_at = NOW(),
                    orgs        = EXCLUDED.orgs
            RETURNING captured_at, orgs
            "#,
        )
        .bind(user_id)
        .bind(orgs_json)
        .fetch_one(&self.pool)
        .await?;
        let (captured_at, orgs) = row;
        Ok(RsiOrgsSnapshot {
            captured_at,
            orgs: orgs.0,
        })
    }

    async fn latest_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Option<RsiOrgsSnapshot>, RsiOrgStoreError> {
        let row: Option<SnapshotRow> =
            sqlx::query_as("SELECT captured_at, orgs FROM rsi_org_snapshots WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(captured_at, orgs)| RsiOrgsSnapshot {
            captured_at,
            orgs: orgs.0,
        }))
    }
}

// -- Test impl + tests -----------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory implementation used by handler-level tests. Mirrors
    /// the Postgres semantics: one snapshot per user, latest-wins on
    /// repeated `save` calls. `captured_at` is stamped at write time
    /// using `Utc::now()` -- the same pattern the Postgres `NOW()`
    /// clause provides.
    #[derive(Default)]
    pub struct MemoryRsiOrgStore {
        snapshots: Mutex<HashMap<Uuid, RsiOrgsSnapshot>>,
    }

    impl MemoryRsiOrgStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl RsiOrgStore for MemoryRsiOrgStore {
        async fn save(
            &self,
            user_id: Uuid,
            orgs: &[RsiOrg],
        ) -> Result<RsiOrgsSnapshot, RsiOrgStoreError> {
            let snapshot = RsiOrgsSnapshot {
                captured_at: Utc::now(),
                orgs: orgs.to_vec(),
            };
            self.snapshots
                .lock()
                .unwrap()
                .insert(user_id, snapshot.clone());
            Ok(snapshot)
        }

        async fn latest_for_user(
            &self,
            user_id: Uuid,
        ) -> Result<Option<RsiOrgsSnapshot>, RsiOrgStoreError> {
            Ok(self.snapshots.lock().unwrap().get(&user_id).cloned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryRsiOrgStore;
    use super::*;

    fn sample_orgs() -> Vec<RsiOrg> {
        vec![
            RsiOrg {
                sid: "IMP".into(),
                name: "Imperium".into(),
                rank: Some("Senior Officer".into()),
                is_main: true,
            },
            RsiOrg {
                sid: "TESTSQDN".into(),
                name: "Test Squadron".into(),
                rank: Some("Recruit".into()),
                is_main: false,
            },
            RsiOrg {
                sid: "FOO".into(),
                name: "Foo Bar".into(),
                rank: None,
                is_main: false,
            },
        ]
    }

    #[tokio::test]
    async fn save_then_latest_round_trips() {
        let store = MemoryRsiOrgStore::new();
        let user = Uuid::new_v4();
        let orgs = sample_orgs();

        let saved = store.save(user, &orgs).await.unwrap();
        assert_eq!(saved.orgs, orgs);

        let got = store.latest_for_user(user).await.unwrap().unwrap();
        assert_eq!(got.orgs, orgs);
        // The store stamps `captured_at` itself; the value we read
        // back must equal the value the save returned, byte-for-byte.
        assert_eq!(got.captured_at, saved.captured_at);
    }

    #[tokio::test]
    async fn save_overwrites_previous_snapshot() {
        let store = MemoryRsiOrgStore::new();
        let user = Uuid::new_v4();

        let first_orgs = vec![RsiOrg {
            sid: "OLDORG".into(),
            name: "Old Org".into(),
            rank: None,
            is_main: true,
        }];
        let first = store.save(user, &first_orgs).await.unwrap();

        // Force a perceptible gap so `captured_at` strictly advances on
        // coarse Windows timers.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        let second_orgs = sample_orgs();
        let second = store.save(user, &second_orgs).await.unwrap();

        // Latest-wins: only the second snapshot is visible. The store
        // does not retain history, so the first org list is gone.
        let got = store.latest_for_user(user).await.unwrap().unwrap();
        assert_eq!(got.orgs, second_orgs);
        assert_ne!(got.orgs, first_orgs);
        assert!(second.captured_at > first.captured_at);
        assert_eq!(got.captured_at, second.captured_at);
    }

    #[tokio::test]
    async fn latest_returns_none_for_unknown_user() {
        let store = MemoryRsiOrgStore::new();
        // Populate the store with one user so we're not just hitting
        // an empty map -- the lookup must filter by user_id.
        let other = Uuid::new_v4();
        store.save(other, &sample_orgs()).await.unwrap();

        let absent = store.latest_for_user(Uuid::new_v4()).await.unwrap();
        assert!(absent.is_none());
    }
}
