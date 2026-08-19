//! Per-widget sharing toggles — Plan 3b Option A.
//!
//! Wires up `users.share_scopes`, a JSONB column added by migration 0021
//! with the explicit intent of gating per-aggregate data for visitors.
//! That wiring never landed; this module finishes the job.
//!
//! ## Shape
//! ```json
//! { "widgets": {
//!     "combat_mission":  false,
//!     "economy":         false,
//!     "travel":          false,
//!     "records":         false,
//!     "recent_activity": false
//! }}
//! ```
//! Default for any missing key: `false` (private). Owners must explicitly
//! opt in to sharing each widget.
//!
//! ## Composition rule
//! - Owner self-read: ignore share_scopes unconditionally.
//! - Visitor read: SpiceDB ReBAC grant required (existing gate, unchanged)
//!   AND `share_scopes.widgets[widget_id] == true`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Per-widget on/off toggles. All fields default to `false` (private).
/// Stored under the `"widgets"` key inside `users.share_scopes` JSONB.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WidgetShareScopes {
    /// Whether visitors can see the Combat & Missions widget.
    #[serde(default)]
    pub combat_mission: bool,
    /// Whether visitors can see the Economy widget.
    #[serde(default)]
    pub economy: bool,
    /// Whether visitors can see the Travel widget.
    #[serde(default)]
    pub travel: bool,
    /// Whether visitors can see the Records widget.
    #[serde(default)]
    pub records: bool,
    /// Whether visitors can see the Recent Activity widget.
    #[serde(default)]
    pub recent_activity: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ShareScopesError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("malformed share_scopes json: {0}")]
    Decode(#[from] serde_json::Error),
}

#[async_trait]
pub trait ShareScopesStore: Send + Sync + 'static {
    /// Read the widget share scopes for `owner_handle`.
    ///
    /// Returns all-false defaults when the `users.share_scopes` column is
    /// NULL, or when the `"widgets"` key is absent. Callers never need to
    /// special-case "no scopes configured".
    async fn get(&self, owner_handle: &str) -> Result<WidgetShareScopes, ShareScopesError>;

    /// Persist new widget share scopes for `owner_handle`.
    ///
    /// Merges only the `"widgets"` sub-key so any other top-level keys in
    /// `share_scopes` are preserved.
    async fn put(
        &self,
        owner_handle: &str,
        scopes: &WidgetShareScopes,
    ) -> Result<(), ShareScopesError>;
}

impl WidgetShareScopes {
    /// Iterate over `(field_name, value)` pairs in a stable order.
    ///
    /// Using this as the single source of truth for widget keys means
    /// adding a 6th field only requires one update here — `render_diff`
    /// and any other consumers pick it up automatically.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, bool)> + '_ {
        [
            ("combat_mission", self.combat_mission),
            ("economy", self.economy),
            ("travel", self.travel),
            ("records", self.records),
            ("recent_activity", self.recent_activity),
        ]
        .into_iter()
    }
}

pub struct PostgresShareScopesStore {
    pool: PgPool,
}

impl PostgresShareScopesStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ShareScopesStore for PostgresShareScopesStore {
    async fn get(&self, owner_handle: &str) -> Result<WidgetShareScopes, ShareScopesError> {
        // Pull the whole `share_scopes` JSONB; extract the `"widgets"` sub-object.
        let row: Option<(Option<serde_json::Value>,)> = sqlx::query_as(
            r#"
            SELECT share_scopes
              FROM users
             WHERE lower(claimed_handle) = lower($1)
             LIMIT 1
            "#,
        )
        .bind(owner_handle)
        .fetch_optional(&self.pool)
        .await?;

        let widgets_val = match row {
            // User not found — return defaults so callers don't special-case.
            None => return Ok(WidgetShareScopes::default()),
            // Column NULL — return defaults.
            Some((None,)) => return Ok(WidgetShareScopes::default()),
            Some((Some(root),)) => {
                // Extract the "widgets" sub-object; default to empty object.
                root.get("widgets")
                    .cloned()
                    .unwrap_or(serde_json::json!({}))
            }
        };

        let scopes: WidgetShareScopes = serde_json::from_value(widgets_val)?;
        Ok(scopes)
    }

    async fn put(
        &self,
        owner_handle: &str,
        scopes: &WidgetShareScopes,
    ) -> Result<(), ShareScopesError> {
        let widgets_json = serde_json::to_value(scopes)?;

        // Merge-patch only the "widgets" sub-key so other top-level keys
        // (e.g. keys added by future migrations) are not overwritten.
        //
        // `jsonb_set(coalesce(share_scopes, '{}'), '{widgets}', $1)` sets
        // the `.widgets` path regardless of whether the column was NULL.
        sqlx::query(
            r#"
            UPDATE users
               SET share_scopes = jsonb_set(
                       COALESCE(share_scopes, '{}'),
                       '{widgets}',
                       $1
                   )
             WHERE lower(claimed_handle) = lower($2)
            "#,
        )
        .bind(widgets_json)
        .bind(owner_handle)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// In-memory store for route-layer unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory [`ShareScopesStore`] for route-layer unit tests.
    /// Mirrors the `MemoryProfileLayoutStore` pattern exactly.
    #[derive(Default)]
    pub struct MemoryShareScopesStore {
        inner: Mutex<HashMap<String, WidgetShareScopes>>,
    }

    #[async_trait]
    impl ShareScopesStore for MemoryShareScopesStore {
        async fn get(&self, owner_handle: &str) -> Result<WidgetShareScopes, ShareScopesError> {
            let inner = self.inner.lock().unwrap();
            Ok(inner
                .get(&owner_handle.to_lowercase())
                .cloned()
                .unwrap_or_default())
        }

        async fn put(
            &self,
            owner_handle: &str,
            scopes: &WidgetShareScopes,
        ) -> Result<(), ShareScopesError> {
            let mut inner = self.inner.lock().unwrap();
            inner.insert(owner_handle.to_lowercase(), scopes.clone());
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Store unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::test_support::MemoryShareScopesStore;
    use super::*;

    #[test]
    fn iter_covers_all_fields_in_stable_order() {
        let scopes = WidgetShareScopes {
            combat_mission: true,
            economy: false,
            travel: true,
            records: false,
            recent_activity: true,
        };
        let pairs: Vec<_> = scopes.iter().collect();
        assert_eq!(pairs.len(), 5, "iter must yield exactly 5 pairs");
        assert_eq!(pairs[0], ("combat_mission", true));
        assert_eq!(pairs[1], ("economy", false));
        assert_eq!(pairs[2], ("travel", true));
        assert_eq!(pairs[3], ("records", false));
        assert_eq!(pairs[4], ("recent_activity", true));
    }

    #[tokio::test]
    async fn get_returns_all_false_when_unset() {
        let store = MemoryShareScopesStore::default();
        let scopes = store.get("alice").await.unwrap();
        assert!(!scopes.combat_mission);
        assert!(!scopes.economy);
        assert!(!scopes.travel);
        assert!(!scopes.records);
        assert!(!scopes.recent_activity);
    }

    #[tokio::test]
    async fn put_then_get_roundtrips() {
        let store = MemoryShareScopesStore::default();
        let scopes = WidgetShareScopes {
            combat_mission: true,
            economy: false,
            travel: true,
            records: false,
            recent_activity: true,
        };
        store.put("alice", &scopes).await.unwrap();
        assert_eq!(store.get("alice").await.unwrap(), scopes);
    }

    #[tokio::test]
    async fn handles_are_case_insensitive() {
        let store = MemoryShareScopesStore::default();
        let scopes = WidgetShareScopes {
            economy: true,
            ..Default::default()
        };
        store.put("Alice", &scopes).await.unwrap();
        let read = store.get("alice").await.unwrap();
        assert!(read.economy);
    }

    #[tokio::test]
    async fn put_overwrites_prior_scopes() {
        let store = MemoryShareScopesStore::default();
        let first = WidgetShareScopes {
            combat_mission: true,
            ..Default::default()
        };
        store.put("alice", &first).await.unwrap();
        let second = WidgetShareScopes {
            economy: true,
            ..Default::default()
        };
        store.put("alice", &second).await.unwrap();
        let read = store.get("alice").await.unwrap();
        assert!(!read.combat_mission, "overwrite should clear prior value");
        assert!(read.economy, "new value should be set");
    }

    #[tokio::test]
    async fn all_enabled_roundtrips() {
        let store = MemoryShareScopesStore::default();
        let scopes = WidgetShareScopes {
            combat_mission: true,
            economy: true,
            travel: true,
            records: true,
            recent_activity: true,
        };
        store.put("bob", &scopes).await.unwrap();
        assert_eq!(store.get("bob").await.unwrap(), scopes);
    }

    #[tokio::test]
    async fn default_is_all_false() {
        let scopes = WidgetShareScopes::default();
        assert!(!scopes.combat_mission);
        assert!(!scopes.economy);
        assert!(!scopes.travel);
        assert!(!scopes.records);
        assert!(!scopes.recent_activity);
    }
}
