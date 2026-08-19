//! Persistent store for RSI citizen-profile snapshots.
//!
//! A snapshot is what we lifted off
//! `https://robertsspaceindustries.com/citizens/{handle}` at the
//! moment the fetch ran (see [`crate::rsi_verify::HttpRsiClient::fetch_profile`]).
//! The store keeps every snapshot. Render paths only read the latest
//! one, but the diff trail is the point: renamed handle, swapped main
//! org, lost badges, all become visible by walking history.
//!
//! The trait fronts the storage so handlers and tests can swap in
//! the in-memory [`test_support::MemoryProfileStore`] without dragging
//! a Postgres pool through every test.
//!
//! Errors collapse into a single [`ProfileStoreError::Backend`] string
//! variant: callers don't need to distinguish "DB down" from "row
//! shape changed" at the route layer (it's a 503 either way), and the
//! single variant matches the noise floor of the existing
//! [`crate::users::UserError`] enum without inheriting its full
//! taxonomy of constraint-mapped errors (we have no unique
//! constraints to map here).

use crate::rsi_verify::Badge;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// One row of the `rsi_profile_snapshots` table. Field shape mirrors
/// [`crate::rsi_verify::RsiProfile`] with `user_id` + `captured_at`
/// added so a row carries enough identity to be looked up directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSnapshot {
    pub user_id: Uuid,
    pub captured_at: DateTime<Utc>,
    pub display_name: Option<String>,
    pub enlistment_date: Option<NaiveDate>,
    pub location: Option<String>,
    pub badges: Vec<Badge>,
    pub bio: Option<String>,
    pub primary_org_summary: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileStoreError {
    /// Anything the storage backend rejected — wrapped sqlx error,
    /// JSON encode failure, etc. The route layer surfaces this as
    /// 503; the message lands in the trace span for ops.
    #[error("profile store backend error: {0}")]
    Backend(String),
}

impl From<sqlx::Error> for ProfileStoreError {
    fn from(err: sqlx::Error) -> Self {
        Self::Backend(err.to_string())
    }
}

#[async_trait]
pub trait ProfileStore: Send + Sync + 'static {
    /// Append a snapshot row. Snapshots are append-only: even if the
    /// upstream profile is unchanged, every successful fetch produces
    /// a row so the audit trail survives. Callers that want
    /// "skip-if-unchanged" must compare to `latest_for_user` first.
    async fn save(&self, snapshot: ProfileSnapshot) -> Result<(), ProfileStoreError>;

    /// Most-recent snapshot for `user_id`. `None` when the user has
    /// never been snapshotted (verified but never refreshed, or every
    /// fetch erred).
    async fn latest_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Option<ProfileSnapshot>, ProfileStoreError>;

    /// Most-recent snapshot for the user whose `claimed_handle`
    /// case-insensitively matches `handle`. Resolves the handle ->
    /// user_id mapping via `users` so callers don't have to.
    /// `None` if the handle isn't claimed or no snapshot exists.
    async fn latest_for_handle(
        &self,
        handle: &str,
    ) -> Result<Option<ProfileSnapshot>, ProfileStoreError>;
}

// -- Postgres impl ---------------------------------------------------

const SNAPSHOT_SELECT: &str = "user_id, captured_at, display_name, enlistment_date, \
                               location, bio, primary_org_summary, badges";

type SnapshotRow = (
    Uuid,
    DateTime<Utc>,
    Option<String>,
    Option<NaiveDate>,
    Option<String>,
    Option<String>,
    Option<String>,
    sqlx::types::Json<Vec<Badge>>,
);

fn snapshot_from_row(row: SnapshotRow) -> ProfileSnapshot {
    let (
        user_id,
        captured_at,
        display_name,
        enlistment_date,
        location,
        bio,
        primary_org_summary,
        badges,
    ) = row;
    ProfileSnapshot {
        user_id,
        captured_at,
        display_name,
        enlistment_date,
        location,
        bio,
        primary_org_summary,
        badges: badges.0,
    }
}

pub struct PostgresProfileStore {
    pool: PgPool,
}

impl PostgresProfileStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProfileStore for PostgresProfileStore {
    async fn save(&self, snapshot: ProfileSnapshot) -> Result<(), ProfileStoreError> {
        // Bind `badges` through `sqlx::types::Json` so serde does the
        // JSONB encoding inline; storing as `serde_json::Value` would
        // round-trip through an extra `to_value` step on every save.
        let badges_json = sqlx::types::Json(&snapshot.badges);
        sqlx::query(
            r#"
            INSERT INTO rsi_profile_snapshots
                (user_id, captured_at, display_name, enlistment_date,
                 location, bio, primary_org_summary, badges)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(snapshot.user_id)
        .bind(snapshot.captured_at)
        .bind(&snapshot.display_name)
        .bind(snapshot.enlistment_date)
        .bind(&snapshot.location)
        .bind(&snapshot.bio)
        .bind(&snapshot.primary_org_summary)
        .bind(badges_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn latest_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Option<ProfileSnapshot>, ProfileStoreError> {
        let sql = format!(
            "SELECT {SNAPSHOT_SELECT} FROM rsi_profile_snapshots \
             WHERE user_id = $1 \
             ORDER BY captured_at DESC LIMIT 1"
        );
        let row: Option<SnapshotRow> = sqlx::query_as(&sql)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(snapshot_from_row))
    }

    async fn latest_for_handle(
        &self,
        handle: &str,
    ) -> Result<Option<ProfileSnapshot>, ProfileStoreError> {
        // The join leans on `users_handle_uq` (unique index on
        // `lower(claimed_handle)`) for the index seek. The DESC PK
        // index on `rsi_profile_snapshots` covers the ORDER BY.
        let sql = format!(
            "SELECT {} FROM rsi_profile_snapshots s \
             JOIN users u ON u.id = s.user_id \
             WHERE lower(u.claimed_handle) = lower($1) \
             ORDER BY s.captured_at DESC LIMIT 1",
            SNAPSHOT_SELECT
                .split(", ")
                .map(|c| format!("s.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let row: Option<SnapshotRow> = sqlx::query_as(&sql)
            .bind(handle)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(snapshot_from_row))
    }
}

// -- Test impl + tests -----------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory implementation used by handler-level tests. Mirrors
    /// the Postgres semantics: append-only writes, latest-wins reads.
    /// `handle_to_user` keeps the handle->user_id mapping the
    /// Postgres impl resolves through the `users` join — tests
    /// register a mapping via [`Self::register_handle`] before
    /// asserting `latest_for_handle`.
    #[derive(Default)]
    pub struct MemoryProfileStore {
        snapshots: Mutex<Vec<ProfileSnapshot>>,
        handle_to_user: Mutex<HashMap<String, Uuid>>,
    }

    impl MemoryProfileStore {
        pub fn new() -> Self {
            Self::default()
        }

        /// Stand in for the `users` join the Postgres impl performs.
        /// Tests register a handle once; subsequent
        /// `latest_for_handle` calls resolve through this map.
        pub fn register_handle(&self, handle: &str, user_id: Uuid) {
            self.handle_to_user
                .lock()
                .unwrap()
                .insert(handle.to_lowercase(), user_id);
        }
    }

    #[async_trait]
    impl ProfileStore for MemoryProfileStore {
        async fn save(&self, snapshot: ProfileSnapshot) -> Result<(), ProfileStoreError> {
            self.snapshots.lock().unwrap().push(snapshot);
            Ok(())
        }

        async fn latest_for_user(
            &self,
            user_id: Uuid,
        ) -> Result<Option<ProfileSnapshot>, ProfileStoreError> {
            let snaps = self.snapshots.lock().unwrap();
            // Latest-wins: scan all rows for the user, pick the one
            // with the greatest `captured_at`. Linear over rows but
            // tests stay tiny.
            Ok(snaps
                .iter()
                .filter(|s| s.user_id == user_id)
                .max_by_key(|s| s.captured_at)
                .cloned())
        }

        async fn latest_for_handle(
            &self,
            handle: &str,
        ) -> Result<Option<ProfileSnapshot>, ProfileStoreError> {
            // Resolve the mapping inside a block so the
            // `MutexGuard` is dropped before we await — without the
            // explicit scope the guard is non-`Send` and the
            // `async-trait` desugar refuses to compile.
            let user_id = {
                let map = self.handle_to_user.lock().unwrap();
                map.get(&handle.to_lowercase()).copied()
            };
            let Some(user_id) = user_id else {
                return Ok(None);
            };
            self.latest_for_user(user_id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryProfileStore;
    use super::*;
    use crate::rsi_verify::Badge;

    fn make_snapshot(user_id: Uuid, captured_at: DateTime<Utc>) -> ProfileSnapshot {
        ProfileSnapshot {
            user_id,
            captured_at,
            display_name: Some("TheCodeSaiyan".to_owned()),
            enlistment_date: NaiveDate::from_ymd_opt(2014, 3, 14),
            location: Some("United Kingdom".to_owned()),
            badges: vec![Badge {
                name: "Original Backer".to_owned(),
                image_url: Some("/badges/founder.png".to_owned()),
            }],
            bio: Some("Hello world.".to_owned()),
            primary_org_summary: Some("Imperium".to_owned()),
        }
    }

    #[tokio::test]
    async fn save_then_latest_for_user_round_trips() {
        let store = MemoryProfileStore::new();
        let user = Uuid::new_v4();
        let earlier = Utc::now() - chrono::Duration::hours(1);
        let later = Utc::now();

        store.save(make_snapshot(user, earlier)).await.unwrap();
        store.save(make_snapshot(user, later)).await.unwrap();

        let latest = store.latest_for_user(user).await.unwrap().unwrap();
        assert_eq!(latest.captured_at, later);
        assert_eq!(latest.display_name.as_deref(), Some("TheCodeSaiyan"));
        assert_eq!(latest.badges.len(), 1);
        assert_eq!(latest.badges[0].name, "Original Backer");

        // Unknown user falls through to None rather than the most
        // recent snapshot for the only known user.
        assert!(store
            .latest_for_user(Uuid::new_v4())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn latest_for_handle_resolves_through_register() {
        let store = MemoryProfileStore::new();
        let user = Uuid::new_v4();
        store.register_handle("TheCodeSaiyan", user);
        store.save(make_snapshot(user, Utc::now())).await.unwrap();

        // Handle is case-insensitive — mirrors the Postgres
        // `lower(claimed_handle)` join condition.
        let by_handle = store
            .latest_for_handle("thecodesaiyan")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_handle.user_id, user);

        // Unknown handle resolves to None rather than erroring.
        assert!(store
            .latest_for_handle("not-a-real-handle")
            .await
            .unwrap()
            .is_none());
    }
}
