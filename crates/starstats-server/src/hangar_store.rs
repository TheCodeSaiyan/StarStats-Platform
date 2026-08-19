//! Persistent store for the user's hangar snapshot (RSI-owned ships).
//!
//! A snapshot is the structured list the desktop tray client lifts off
//! `https://robertsspaceindustries.com/account/pledges` while logged in
//! against the user's RSI session cookie -- the cookie itself never
//! leaves the user's machine; only the parsed shape arrives here.
//!
//! Unlike [`crate::profile_store`], the hangar store keeps **only the
//! latest snapshot per user**. History is intentionally out of scope:
//! the dashboard renders "what you currently own", and an unbounded
//! per-user history table would be the largest single-table growth
//! vector in the system (~50 rows per push). If hangar diffing ever
//! becomes a feature it gets its own append-only table.
//!
//! Mirrors [`crate::profile_store`] in shape: trait-fronted store with
//! an in-memory test impl, single [`HangarStoreError::Backend`] variant
//! that collapses sqlx + JSON encode failures into a 503 at the route
//! layer.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use starstats_core::wire::HangarShip;
use uuid::Uuid;

/// One row of the `hangar_snapshots` table. Returned from the store —
/// distinct from [`starstats_core::wire::HangarPushRequest`] (the
/// inbound wire payload).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct HangarSnapshot {
    pub captured_at: DateTime<Utc>,
    /// Ships owned by the user at `captured_at`. The OpenAPI schema
    /// points at `HangarShipSchema` (the `utoipa`-aware mirror in
    /// `hangar_routes`) because `HangarShip` itself lives in the core
    /// crate, which carries no `utoipa` dep.
    #[schema(value_type = Vec<crate::hangar_routes::HangarShipSchema>)]
    pub ships: Vec<HangarShip>,
}

#[derive(Debug, thiserror::Error)]
pub enum HangarStoreError {
    /// Anything the storage backend rejected -- wrapped sqlx error,
    /// JSON encode failure, etc. The route layer surfaces this as
    /// 503; the message lands in the trace span for ops.
    #[error("hangar store backend error: {0}")]
    Backend(String),
}

impl From<sqlx::Error> for HangarStoreError {
    fn from(err: sqlx::Error) -> Self {
        Self::Backend(err.to_string())
    }
}

#[async_trait]
pub trait HangarStore: Send + Sync + 'static {
    /// Replace the user's current hangar snapshot. The server stamps
    /// `captured_at` itself; pass the wire-format `ships` slice.
    async fn put_snapshot(
        &self,
        user_id: Uuid,
        ships: &[HangarShip],
    ) -> Result<HangarSnapshot, HangarStoreError>;

    /// Fetch the latest snapshot for the user, if any.
    async fn get_snapshot(&self, user_id: Uuid)
        -> Result<Option<HangarSnapshot>, HangarStoreError>;
}

// -- Postgres impl ---------------------------------------------------

type SnapshotRow = (DateTime<Utc>, sqlx::types::Json<Vec<HangarShip>>);

pub struct PostgresHangarStore {
    pool: PgPool,
}

impl PostgresHangarStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HangarStore for PostgresHangarStore {
    async fn put_snapshot(
        &self,
        user_id: Uuid,
        ships: &[HangarShip],
    ) -> Result<HangarSnapshot, HangarStoreError> {
        // Bind `ships` through `sqlx::types::Json` so serde does the
        // JSONB encoding inline -- mirrors `profile_store::save`. The
        // ON CONFLICT clause makes the upsert atomic; `captured_at` is
        // refreshed to NOW() on every push so a "force refresh" from
        // the UI bumps the timestamp even if the ship list is identical.
        let ships_json = sqlx::types::Json(ships);
        let row: SnapshotRow = sqlx::query_as(
            r#"
            INSERT INTO hangar_snapshots (user_id, captured_at, ships)
            VALUES ($1, NOW(), $2)
            ON CONFLICT (user_id) DO UPDATE
                SET captured_at = NOW(),
                    ships       = EXCLUDED.ships
            RETURNING captured_at, ships
            "#,
        )
        .bind(user_id)
        .bind(ships_json)
        .fetch_one(&self.pool)
        .await?;
        let (captured_at, ships) = row;
        Ok(HangarSnapshot {
            captured_at,
            ships: ships.0,
        })
    }

    async fn get_snapshot(
        &self,
        user_id: Uuid,
    ) -> Result<Option<HangarSnapshot>, HangarStoreError> {
        let row: Option<SnapshotRow> =
            sqlx::query_as("SELECT captured_at, ships FROM hangar_snapshots WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(captured_at, ships)| HangarSnapshot {
            captured_at,
            ships: ships.0,
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
    /// repeated `put_snapshot` calls. `captured_at` is stamped at
    /// write time using `Utc::now()` -- the same pattern the Postgres
    /// `NOW()` clause provides.
    #[derive(Default)]
    pub struct MemoryHangarStore {
        snapshots: Mutex<HashMap<Uuid, HangarSnapshot>>,
    }

    impl MemoryHangarStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl HangarStore for MemoryHangarStore {
        async fn put_snapshot(
            &self,
            user_id: Uuid,
            ships: &[HangarShip],
        ) -> Result<HangarSnapshot, HangarStoreError> {
            let snapshot = HangarSnapshot {
                captured_at: Utc::now(),
                ships: ships.to_vec(),
            };
            self.snapshots
                .lock()
                .unwrap()
                .insert(user_id, snapshot.clone());
            Ok(snapshot)
        }

        async fn get_snapshot(
            &self,
            user_id: Uuid,
        ) -> Result<Option<HangarSnapshot>, HangarStoreError> {
            Ok(self.snapshots.lock().unwrap().get(&user_id).cloned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryHangarStore;
    use super::*;

    fn sample_ships() -> Vec<HangarShip> {
        vec![
            HangarShip {
                name: "Aegis Avenger Titan".into(),
                manufacturer: Some("Aegis Dynamics".into()),
                pledge_id: Some("12345678".into()),
                kind: Some("ship".into()),
                contains: vec![],
            },
            HangarShip {
                name: "Greycat PTV".into(),
                manufacturer: None,
                pledge_id: None,
                kind: None,
                contains: vec![],
            },
        ]
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let store = MemoryHangarStore::new();
        let user = Uuid::new_v4();
        let ships = sample_ships();

        let put = store.put_snapshot(user, &ships).await.unwrap();
        assert_eq!(put.ships, ships);

        let got = store.get_snapshot(user).await.unwrap().unwrap();
        assert_eq!(got.ships, ships);
        // The store stamps `captured_at` itself; the value we read
        // back must equal the value the put returned, byte-for-byte.
        assert_eq!(got.captured_at, put.captured_at);
    }

    #[tokio::test]
    async fn put_overwrites_previous_snapshot() {
        let store = MemoryHangarStore::new();
        let user = Uuid::new_v4();

        let first_ships = vec![HangarShip {
            name: "Old Ship".into(),
            manufacturer: None,
            pledge_id: None,
            kind: None,
            contains: vec![],
        }];
        let first = store.put_snapshot(user, &first_ships).await.unwrap();

        // Force a perceptible gap so `captured_at` strictly advances on
        // coarse Windows timers.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        let second_ships = sample_ships();
        let second = store.put_snapshot(user, &second_ships).await.unwrap();

        // Latest-wins: only the second snapshot is visible. The store
        // does not retain history, so the first ship list is gone.
        let got = store.get_snapshot(user).await.unwrap().unwrap();
        assert_eq!(got.ships, second_ships);
        assert_ne!(got.ships, first_ships);
        assert!(second.captured_at > first.captured_at);
        assert_eq!(got.captured_at, second.captured_at);
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown_user() {
        let store = MemoryHangarStore::new();
        // Populate the store with one user so we're not just hitting
        // an empty map -- the lookup must filter by user_id.
        let other = Uuid::new_v4();
        store.put_snapshot(other, &sample_ships()).await.unwrap();

        let absent = store.get_snapshot(Uuid::new_v4()).await.unwrap();
        assert!(absent.is_none());
    }
}
