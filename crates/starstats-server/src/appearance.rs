//! Sitewide appearance defaults: today, just the theme-switch wave
//! animation speed. Singleton row (migration 0052), mirroring the
//! `WaitlistStore` cap/gate shape in `waitlist.rs` -- a Postgres impl
//! for production and a `MemoryAppearanceStore` under `test_support`
//! for handler-level tests.
//!
//! Resolution order (web-side, not here): per-user preference
//! (`users.preferences ->> 'theme_wave_speed'`) wins when set, else
//! this sitewide default, else `normal`. This module only owns the
//! sitewide half.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

/// Default wave speed when nothing has ever been written -- matches
/// the migration's column default so a fresh Memory store and a
/// freshly-migrated Postgres database agree. Only `MemoryAppearanceStore`
/// (test-only) reads this today; the Postgres impl gets its default from
/// the migration's `DEFAULT 'normal'` clause instead. `#[cfg(test)]`
/// because clippy's dead-code lint (rightly) flags an unused pub const
/// under `--all-targets` otherwise.
#[cfg(test)]
pub const DEFAULT_WAVE_SPEED: &str = "normal";

#[derive(Debug, thiserror::Error)]
pub enum AppearanceError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait AppearanceStore: Send + Sync + 'static {
    async fn get_wave_speed(&self) -> Result<String, AppearanceError>;
    async fn set_wave_speed(
        &self,
        speed: &str,
        updated_by: Option<Uuid>,
    ) -> Result<(), AppearanceError>;
}

pub struct PostgresAppearanceStore {
    pool: PgPool,
}

impl PostgresAppearanceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AppearanceStore for PostgresAppearanceStore {
    async fn get_wave_speed(&self) -> Result<String, AppearanceError> {
        let (speed,): (String,) =
            sqlx::query_as("SELECT theme_wave_speed FROM appearance_config WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        Ok(speed)
    }

    async fn set_wave_speed(
        &self,
        speed: &str,
        updated_by: Option<Uuid>,
    ) -> Result<(), AppearanceError> {
        sqlx::query(
            "UPDATE appearance_config SET theme_wave_speed = $1, updated_at = NOW(), \
             updated_by = $2 WHERE id = 1",
        )
        .bind(speed)
        .bind(updated_by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// In-memory `AppearanceStore` for handler and store tests. Starts
    /// at `DEFAULT_WAVE_SPEED`, matching the migration's column
    /// default + seed row.
    pub struct MemoryAppearanceStore {
        speed: Mutex<String>,
    }

    impl Default for MemoryAppearanceStore {
        fn default() -> Self {
            Self {
                speed: Mutex::new(DEFAULT_WAVE_SPEED.to_string()),
            }
        }
    }

    impl MemoryAppearanceStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl AppearanceStore for MemoryAppearanceStore {
        async fn get_wave_speed(&self) -> Result<String, AppearanceError> {
            Ok(self.speed.lock().unwrap().clone())
        }

        async fn set_wave_speed(
            &self,
            speed: &str,
            _updated_by: Option<Uuid>,
        ) -> Result<(), AppearanceError> {
            *self.speed.lock().unwrap() = speed.to_string();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryAppearanceStore;
    use super::*;

    // -- Test 1: default is 'normal' ----------------------------------

    #[tokio::test]
    async fn default_wave_speed_is_normal() {
        let store = MemoryAppearanceStore::new();
        assert_eq!(store.get_wave_speed().await.unwrap(), "normal");
    }

    // -- Test 2: set/get round-trip ------------------------------------

    #[tokio::test]
    async fn set_then_get_round_trips() {
        let store = MemoryAppearanceStore::new();
        store.set_wave_speed("fast", None).await.unwrap();
        assert_eq!(store.get_wave_speed().await.unwrap(), "fast");
    }

    // -- Test 3: every allowed value round-trips -----------------------

    #[tokio::test]
    async fn every_allowed_speed_round_trips() {
        let store = MemoryAppearanceStore::new();
        for speed in ["off", "slow", "normal", "fast"] {
            store.set_wave_speed(speed, None).await.unwrap();
            assert_eq!(store.get_wave_speed().await.unwrap(), speed);
        }
    }

    // -- Test 4: repeated writes overwrite, not accumulate -------------

    #[tokio::test]
    async fn repeated_writes_overwrite_the_prior_value() {
        let store = MemoryAppearanceStore::new();
        store.set_wave_speed("slow", None).await.unwrap();
        store.set_wave_speed("fast", None).await.unwrap();
        assert_eq!(store.get_wave_speed().await.unwrap(), "fast");
    }

    // -- Test 5: updated_by is accepted but doesn't affect the read ----

    #[tokio::test]
    async fn updated_by_is_accepted_without_changing_the_read_value() {
        let store = MemoryAppearanceStore::new();
        let admin = Uuid::new_v4();
        store.set_wave_speed("slow", Some(admin)).await.unwrap();
        assert_eq!(store.get_wave_speed().await.unwrap(), "slow");
    }

    // -- Test 6: independent store instances don't share state ---------

    #[tokio::test]
    async fn independent_instances_do_not_share_state() {
        let a = MemoryAppearanceStore::new();
        let b = MemoryAppearanceStore::new();
        a.set_wave_speed("fast", None).await.unwrap();
        assert_eq!(b.get_wave_speed().await.unwrap(), "normal");
    }

    // -- Test 7: setting the same value twice is a stable no-op --------

    #[tokio::test]
    async fn setting_the_same_value_twice_is_stable() {
        let store = MemoryAppearanceStore::new();
        store.set_wave_speed("off", None).await.unwrap();
        store.set_wave_speed("off", None).await.unwrap();
        assert_eq!(store.get_wave_speed().await.unwrap(), "off");
    }
}
