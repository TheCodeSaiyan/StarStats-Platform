//! DB-backed, admin-managed runtime config for Ship Matrix enrichment.
//!
//! Singleton row (`ship_matrix_config.id = 1`, enforced by a `CHECK` in
//! migration `0042_ship_matrix_config.sql`), mirroring the SMTP
//! admin-config pattern in [`crate::smtp_config_store`].
//!
//! Currently holds one knob: `media_enabled` — the comply-on-request
//! kill-switch for surfacing RSI ship images. It replaces the old
//! `STARSTATS_SHIP_MATRIX_MEDIA` env var so an admin can flip it from the
//! UI with no redeploy. The value is mirrored into an in-memory
//! [`std::sync::atomic::AtomicBool`] at boot and on every admin write, so
//! the hot path (the media proxy) never hits the DB per request.

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

#[async_trait]
pub trait ShipMatrixConfigStore: Send + Sync + 'static {
    /// Read the media kill-switch. The migration guarantees the row is
    /// always present (seeded `media_enabled = false`).
    async fn get_media_enabled(&self) -> Result<bool>;

    /// Persist the media kill-switch. `updated_by` is the admin user id
    /// (audit), `None` for a system-driven write.
    async fn set_media_enabled(&self, enabled: bool, updated_by: Option<Uuid>) -> Result<()>;
}

pub struct PostgresShipMatrixConfigStore {
    pool: PgPool,
}

impl PostgresShipMatrixConfigStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ShipMatrixConfigStore for PostgresShipMatrixConfigStore {
    async fn get_media_enabled(&self) -> Result<bool> {
        let row: (bool,) =
            sqlx::query_as("SELECT media_enabled FROM ship_matrix_config WHERE id = 1")
                .fetch_one(&self.pool)
                .await
                .context("read ship_matrix_config")?;
        Ok(row.0)
    }

    async fn set_media_enabled(&self, enabled: bool, updated_by: Option<Uuid>) -> Result<()> {
        sqlx::query(
            "UPDATE ship_matrix_config \
                 SET media_enabled = $1, updated_at = now(), updated_by = $2 \
               WHERE id = 1",
        )
        .bind(enabled)
        .bind(updated_by)
        .execute(&self.pool)
        .await
        .context("write ship_matrix_config")?;
        Ok(())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// In-memory [`ShipMatrixConfigStore`] for handler/store tests.
    #[derive(Default)]
    pub struct MemoryShipMatrixConfigStore {
        media_enabled: Mutex<bool>,
    }

    impl MemoryShipMatrixConfigStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl ShipMatrixConfigStore for MemoryShipMatrixConfigStore {
        async fn get_media_enabled(&self) -> Result<bool> {
            Ok(*self.media_enabled.lock().unwrap())
        }

        async fn set_media_enabled(&self, enabled: bool, _updated_by: Option<Uuid>) -> Result<()> {
            *self.media_enabled.lock().unwrap() = enabled;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryShipMatrixConfigStore;
    use super::*;

    #[tokio::test]
    async fn defaults_to_disabled() {
        let store = MemoryShipMatrixConfigStore::new();
        assert!(!store.get_media_enabled().await.unwrap());
    }

    #[tokio::test]
    async fn set_then_get_round_trips() {
        let store = MemoryShipMatrixConfigStore::new();
        store.set_media_enabled(true, None).await.unwrap();
        assert!(store.get_media_enabled().await.unwrap());
        store.set_media_enabled(false, None).await.unwrap();
        assert!(!store.get_media_enabled().await.unwrap());
    }
}
