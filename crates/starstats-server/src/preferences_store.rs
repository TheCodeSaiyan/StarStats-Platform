//! Persistent store for the user's UI preferences (theme + future
//! forward-extensible toggles).
//!
//! Preferences live on the existing `users` row as a JSONB column
//! (`users.preferences`, default `'{}'::jsonb`) rather than a separate
//! table because:
//!
//!  - The set is small and per-user (theme today; notification toggles
//!    + accent intensity + name plate later) — a satellite table would
//!    be all overhead.
//!  - The `users` row already exists by the time anyone calls these
//!    endpoints (auth gate guarantees it), so PUT is a plain UPDATE
//!    and never has to INSERT.
//!  - JSONB lets us evolve the schema without a migration per field.
//!
//! Mirrors the trait-fronted shape of [`crate::hangar_store`]: a
//! Postgres impl for production and a [`MemoryPreferencesStore`] under
//! `test_support` for handler-level tests. A single
//! [`sqlx::Error`] is surfaced to the route layer as a 500.

use async_trait::async_trait;
use sqlx::PgPool;
use starstats_core::wire::UserPreferences;
use uuid::Uuid;

#[async_trait]
pub trait PreferencesStore: Send + Sync + 'static {
    /// Fetch the caller's preferences. Returns `UserPreferences::default()`
    /// when the column is `'{}'::jsonb` or has no fields set — callers
    /// never have to special-case "no row stored".
    async fn get(&self, user_id: Uuid) -> Result<UserPreferences, sqlx::Error>;

    /// Sparse-merge the caller's preferences. Fields present in `prefs`
    /// replace stored values; absent fields are left untouched. The
    /// nested `remote_sync` struct is merged field-by-field as well.
    /// The user row is guaranteed to exist (auth gate), so this is a
    /// plain UPDATE rather than an upsert.
    async fn put(&self, user_id: Uuid, prefs: &UserPreferences) -> Result<(), sqlx::Error>;
}

// -- Postgres impl ---------------------------------------------------

pub struct PostgresPreferencesStore {
    pool: PgPool,
}

impl PostgresPreferencesStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PreferencesStore for PostgresPreferencesStore {
    async fn get(&self, user_id: Uuid) -> Result<UserPreferences, sqlx::Error> {
        // `sqlx::types::Json<UserPreferences>` decodes the JSONB column
        // through serde — same pattern as `hangar_store::get_snapshot`.
        // A row that doesn't exist (deleted account, racing handlers)
        // is returned as `RowNotFound` and propagated; the route maps
        // that to 500. An empty `'{}'` JSONB decodes cleanly to
        // `UserPreferences::default()` because every field is optional.
        let row: (sqlx::types::Json<UserPreferences>,) =
            sqlx::query_as("SELECT preferences FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0 .0)
    }

    async fn put(&self, user_id: Uuid, prefs: &UserPreferences) -> Result<(), sqlx::Error> {
        // Sparse-merge semantics: fields present in `prefs` replace
        // stored values; absent fields are left untouched. Postgres'
        // `||` operator on JSONB merges top-level keys (right side
        // wins). For nested fields like `remote_sync`, we
        // additionally merge the inner object so a PUT of
        // `{"remote_sync": {"batch_size": 500}}` only touches
        // batch_size and not the rest of the lane.
        //
        // `jsonb_strip_nulls` removes any explicitly-null leaves the
        // caller sent — note: when the caller sends `"theme": null`
        // they mean "clear theme", which jsonb_strip_nulls would
        // suppress. To support explicit clearing we strip nulls only
        // from the deep-merge product, not from the incoming payload:
        // null at the top level falls through `||` and clears the key.
        //
        // Implementation: a small SQL function-ish chain. The
        // `coalesce(stored, '{}'::jsonb)` defends against a
        // surprise NULL in the column (shouldn't happen given the
        // migration's NOT NULL DEFAULT but cheap insurance).
        let body = serde_json::to_value(prefs).map_err(|e| {
            sqlx::Error::Decode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            )))
        })?;
        sqlx::query(
            r#"
            UPDATE users
            SET preferences = (
                CASE
                    WHEN $1::jsonb -> 'remote_sync' IS NOT NULL THEN
                        (coalesce(preferences, '{}'::jsonb) || $1::jsonb)
                        || jsonb_build_object(
                            'remote_sync',
                            coalesce(preferences -> 'remote_sync', '{}'::jsonb)
                              || ($1::jsonb -> 'remote_sync')
                        )
                    ELSE
                        coalesce(preferences, '{}'::jsonb) || $1::jsonb
                END
            )
            WHERE id = $2
            "#,
        )
        .bind(&body)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

// -- Test impl + tests -----------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory implementation used by handler-level tests. Mirrors
    /// the Postgres semantics: GET returns `UserPreferences::default()`
    /// when nothing is stored (the production code reads `'{}'::jsonb`
    /// which decodes to default), and PUT replaces in full.
    #[derive(Default)]
    pub struct MemoryPreferencesStore {
        prefs: Mutex<HashMap<Uuid, UserPreferences>>,
    }

    impl MemoryPreferencesStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl PreferencesStore for MemoryPreferencesStore {
        async fn get(&self, user_id: Uuid) -> Result<UserPreferences, sqlx::Error> {
            Ok(self
                .prefs
                .lock()
                .unwrap()
                .get(&user_id)
                .cloned()
                .unwrap_or_default())
        }

        async fn put(&self, user_id: Uuid, prefs: &UserPreferences) -> Result<(), sqlx::Error> {
            // Mirror the Postgres sparse-merge semantics: fields present
            // in `prefs` replace stored values; absent fields are left
            // untouched. For `remote_sync`, merge the inner struct
            // field-by-field so a PUT touching only `batch_size`
            // doesn't clobber the rest of the lane.
            let mut guard = self.prefs.lock().unwrap();
            let entry = guard.entry(user_id).or_default();
            if prefs.theme.is_some() {
                entry.theme = prefs.theme.clone();
            }
            if prefs.debug_logging.is_some() {
                entry.debug_logging = prefs.debug_logging;
            }
            if prefs.auto_update_check.is_some() {
                entry.auto_update_check = prefs.auto_update_check;
            }
            if prefs.release_channel.is_some() {
                entry.release_channel = prefs.release_channel.clone();
            }
            if prefs.api_url.is_some() {
                entry.api_url = prefs.api_url.clone();
            }
            if prefs.kb_view.is_some() {
                entry.kb_view = prefs.kb_view.clone();
            }
            if prefs.kb_units.is_some() {
                entry.kb_units = prefs.kb_units.clone();
            }
            if prefs.theme_wave_speed.is_some() {
                entry.theme_wave_speed = prefs.theme_wave_speed.clone();
            }
            if let Some(incoming) = &prefs.remote_sync {
                let target = entry.remote_sync.get_or_insert_with(Default::default);
                if incoming.enabled.is_some() {
                    target.enabled = incoming.enabled;
                }
                if incoming.priority_interval_secs.is_some() {
                    target.priority_interval_secs = incoming.priority_interval_secs;
                }
                if incoming.interval_secs.is_some() {
                    target.interval_secs = incoming.interval_secs;
                }
                if incoming.batch_size.is_some() {
                    target.batch_size = incoming.batch_size;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryPreferencesStore;
    use super::*;
    use starstats_core::wire::RemoteSyncPrefs;

    #[tokio::test]
    async fn get_returns_default_when_nothing_stored() {
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();

        let got = store.get(user).await.unwrap();
        assert_eq!(got, UserPreferences::default());
        assert!(got.theme.is_none());
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();
        let prefs = UserPreferences {
            theme: Some("pyro".into()),
            ..UserPreferences::default()
        };

        store.put(user, &prefs).await.unwrap();
        let got = store.get(user).await.unwrap();
        assert_eq!(got, prefs);
    }

    #[tokio::test]
    async fn absent_fields_preserved_on_partial_put() {
        // Seed with two fields.
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();
        store
            .put(
                user,
                &UserPreferences {
                    theme: Some("pyro".into()),
                    debug_logging: Some(true),
                    ..UserPreferences::default()
                },
            )
            .await
            .unwrap();

        // PUT only `theme` → debug_logging must survive.
        store
            .put(
                user,
                &UserPreferences {
                    theme: Some("nyx".into()),
                    ..UserPreferences::default()
                },
            )
            .await
            .unwrap();

        let got = store.get(user).await.unwrap();
        assert_eq!(got.theme.as_deref(), Some("nyx"));
        assert_eq!(got.debug_logging, Some(true));
    }

    #[tokio::test]
    async fn empty_put_is_a_noop() {
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();
        store
            .put(
                user,
                &UserPreferences {
                    theme: Some("terra".into()),
                    ..UserPreferences::default()
                },
            )
            .await
            .unwrap();
        // Empty body → leave everything alone.
        store.put(user, &UserPreferences::default()).await.unwrap();

        let got = store.get(user).await.unwrap();
        assert_eq!(got.theme.as_deref(), Some("terra"));
    }

    #[tokio::test]
    async fn nested_remote_sync_merges_field_by_field() {
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();
        store
            .put(
                user,
                &UserPreferences {
                    remote_sync: Some(RemoteSyncPrefs {
                        enabled: Some(true),
                        priority_interval_secs: Some(5),
                        interval_secs: Some(60),
                        batch_size: Some(200),
                    }),
                    ..UserPreferences::default()
                },
            )
            .await
            .unwrap();

        // PUT only batch_size → the other three must survive.
        store
            .put(
                user,
                &UserPreferences {
                    remote_sync: Some(RemoteSyncPrefs {
                        batch_size: Some(500),
                        ..RemoteSyncPrefs::default()
                    }),
                    ..UserPreferences::default()
                },
            )
            .await
            .unwrap();

        let got = store.get(user).await.unwrap();
        let rs = got.remote_sync.expect("remote_sync should survive");
        assert_eq!(rs.enabled, Some(true));
        assert_eq!(rs.priority_interval_secs, Some(5));
        assert_eq!(rs.interval_secs, Some(60));
        assert_eq!(rs.batch_size, Some(500));
    }
}
