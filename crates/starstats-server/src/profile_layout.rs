//! Owner-side profile-layout storage.
//!
//! Persists which widgets appear on `/u/[handle]`, in what order,
//! enabled or not, compact or expanded. NULL in `users.profile_layout`
//! means "owner hasn't customised yet; use DEFAULT_LAYOUT from the
//! web layer." The store knows nothing about defaults — that's a web
//! concern.
//!
//! Validation (size enum, id length, max entries) happens in the HTTP
//! handler before this trait is called.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Single widget entry. `id` is a stable string identifier (see the
/// web-side registry); the server does NOT enforce registry membership
/// — unknown ids are filtered at render time by the web app, so
/// schema evolution stays additive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LayoutEntry {
    /// Widget id (e.g. "sessions", "heatmap"). 1..=64 ASCII chars.
    pub id: String,
    /// Whether this widget renders for visitors when sharing allows.
    pub enabled: bool,
    /// Display size at render time.
    pub size: WidgetSize,
    /// Free-grid column origin (0-based) on the 24-col dashboard grid.
    /// Absent on legacy layouts saved before the drag/resize grid (M7);
    /// the web layer derives a position from order + span when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<u32>,
    /// Free-grid row origin (0-based). See `x`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<u32>,
    /// Free-grid width in columns (1..=24). See `x`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    /// Free-grid height in rows. See `x`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum WidgetSize {
    Compact,
    Expanded,
}

impl WidgetSize {
    pub fn as_str(self) -> &'static str {
        match self {
            WidgetSize::Compact => "compact",
            WidgetSize::Expanded => "expanded",
        }
    }
}

/// Which widget-layout surface a request targets. Each surface is an
/// independent column on `users`. Defaults to `Profile` so existing
/// callers (and the original /v1 route shape) are unchanged.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum LayoutSurface {
    /// Public profile at `/u/[handle]` — column `profile_layout`.
    #[default]
    Profile,
    /// Private home at `/me` — column `home_layout`.
    Home,
}

impl LayoutSurface {
    /// The `users` column backing this surface. Hardcoded `&'static str`
    /// (NEVER user input) — safe to interpolate into SQL.
    pub fn column(self) -> &'static str {
        match self {
            LayoutSurface::Profile => "profile_layout",
            LayoutSurface::Home => "home_layout",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileLayoutError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("layout is malformed json: {0}")]
    Decode(#[from] serde_json::Error),
}

#[async_trait]
pub trait ProfileLayoutStore: Send + Sync + 'static {
    /// Returns `None` if the user row has NULL in the surface's column.
    /// Callers fall back to their default in that case.
    async fn get(
        &self,
        surface: LayoutSurface,
        owner_handle: &str,
    ) -> Result<Option<Vec<LayoutEntry>>, ProfileLayoutError>;

    /// Overwrites the entire layout array for the surface. Empty array
    /// IS valid (owner has disabled every widget); pass `None` to clear
    /// back to "use the default".
    async fn put(
        &self,
        surface: LayoutSurface,
        owner_handle: &str,
        layout: Option<&[LayoutEntry]>,
    ) -> Result<(), ProfileLayoutError>;
}

pub struct PostgresProfileLayoutStore {
    pool: PgPool,
}

impl PostgresProfileLayoutStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProfileLayoutStore for PostgresProfileLayoutStore {
    async fn get(
        &self,
        surface: LayoutSurface,
        owner_handle: &str,
    ) -> Result<Option<Vec<LayoutEntry>>, ProfileLayoutError> {
        // Handles are stored case-preserved; lookups are case-insensitive.
        // surface.column() is a hardcoded &'static str, never user input.
        let sql = format!(
            "SELECT {} FROM users WHERE lower(claimed_handle) = lower($1) LIMIT 1",
            surface.column(),
        );
        let row: Option<(Option<serde_json::Value>,)> = sqlx::query_as(&sql)
            .bind(owner_handle)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            None => Ok(None),          // user not found
            Some((None,)) => Ok(None), // user found, column NULL
            Some((Some(value),)) => {
                let layout: Vec<LayoutEntry> = serde_json::from_value(value)?;
                Ok(Some(layout))
            }
        }
    }

    async fn put(
        &self,
        surface: LayoutSurface,
        owner_handle: &str,
        layout: Option<&[LayoutEntry]>,
    ) -> Result<(), ProfileLayoutError> {
        let json_value = match layout {
            Some(entries) => Some(serde_json::to_value(entries)?),
            None => None,
        };
        // surface.column() is a hardcoded &'static str, never user input.
        let sql = format!(
            "UPDATE users SET {} = $1 WHERE lower(claimed_handle) = lower($2)",
            surface.column(),
        );
        sqlx::query(&sql)
            .bind(json_value)
            .bind(owner_handle)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory impl for route-layer unit tests. Mirrors the
    /// `share_metadata::test_support::MemoryShareMetadataStore` pattern.
    #[derive(Default)]
    pub struct MemoryProfileLayoutStore {
        inner: Mutex<HashMap<(LayoutSurface, String), Vec<LayoutEntry>>>,
    }

    #[async_trait]
    impl ProfileLayoutStore for MemoryProfileLayoutStore {
        async fn get(
            &self,
            surface: LayoutSurface,
            owner_handle: &str,
        ) -> Result<Option<Vec<LayoutEntry>>, ProfileLayoutError> {
            let inner = self.inner.lock().unwrap();
            Ok(inner.get(&(surface, owner_handle.to_lowercase())).cloned())
        }

        async fn put(
            &self,
            surface: LayoutSurface,
            owner_handle: &str,
            layout: Option<&[LayoutEntry]>,
        ) -> Result<(), ProfileLayoutError> {
            let mut inner = self.inner.lock().unwrap();
            let key = (surface, owner_handle.to_lowercase());
            match layout {
                Some(entries) => {
                    inner.insert(key, entries.to_vec());
                }
                None => {
                    inner.remove(&key);
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryProfileLayoutStore;
    use super::*;

    fn entry(id: &str) -> LayoutEntry {
        LayoutEntry {
            id: id.to_string(),
            enabled: true,
            size: WidgetSize::Compact,
            x: None,
            y: None,
            w: None,
            h: None,
        }
    }

    #[test]
    fn legacy_json_without_geometry_still_deserializes() {
        // Layouts stored before M7 carry only {id,enabled,size}. The new
        // optional geometry fields must default to None (backward compat).
        let legacy = serde_json::json!({
            "id": "sessions",
            "enabled": true,
            "size": "compact"
        });
        let parsed: LayoutEntry = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.x, None);
        assert_eq!(parsed.h, None);
    }

    #[test]
    fn geometry_round_trips_and_is_omitted_when_absent() {
        // Absent geometry serializes to just {id,enabled,size} (tiny,
        // byte-identical to legacy) thanks to skip_serializing_if.
        let bare = entry("orgs");
        let json = serde_json::to_value(&bare).unwrap();
        assert!(json.get("x").is_none());
        assert_eq!(json.as_object().unwrap().len(), 3);

        // Present geometry round-trips.
        let positioned = LayoutEntry {
            id: "heatmap".to_string(),
            enabled: true,
            size: WidgetSize::Expanded,
            x: Some(6),
            y: Some(0),
            w: Some(24),
            h: Some(8),
        };
        let back: LayoutEntry =
            serde_json::from_value(serde_json::to_value(&positioned).unwrap()).unwrap();
        assert_eq!(back, positioned);
    }

    #[tokio::test]
    async fn get_returns_none_when_unset() {
        let store = MemoryProfileLayoutStore::default();
        assert_eq!(
            store.get(LayoutSurface::Profile, "alice").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn put_then_get_roundtrips() {
        let store = MemoryProfileLayoutStore::default();
        let layout = vec![entry("sessions"), entry("heatmap")];
        store
            .put(LayoutSurface::Profile, "alice", Some(&layout))
            .await
            .unwrap();
        assert_eq!(
            store.get(LayoutSurface::Profile, "alice").await.unwrap(),
            Some(layout)
        );
    }

    #[tokio::test]
    async fn put_none_clears_back_to_default() {
        let store = MemoryProfileLayoutStore::default();
        store
            .put(LayoutSurface::Profile, "alice", Some(&[entry("sessions")]))
            .await
            .unwrap();
        store
            .put(LayoutSurface::Profile, "alice", None)
            .await
            .unwrap();
        assert_eq!(
            store.get(LayoutSurface::Profile, "alice").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn handles_are_case_insensitive() {
        let store = MemoryProfileLayoutStore::default();
        store
            .put(LayoutSurface::Profile, "Alice", Some(&[entry("sessions")]))
            .await
            .unwrap();
        assert_eq!(
            store.get(LayoutSurface::Profile, "alice").await.unwrap(),
            Some(vec![entry("sessions")]),
        );
    }

    #[tokio::test]
    async fn empty_layout_is_valid() {
        let store = MemoryProfileLayoutStore::default();
        store
            .put(LayoutSurface::Profile, "alice", Some(&[]))
            .await
            .unwrap();
        assert_eq!(
            store.get(LayoutSurface::Profile, "alice").await.unwrap(),
            Some(vec![])
        );
    }

    #[tokio::test]
    async fn put_overwrites_prior_layout() {
        let store = MemoryProfileLayoutStore::default();
        store
            .put(LayoutSurface::Profile, "alice", Some(&[entry("sessions")]))
            .await
            .unwrap();
        let next = vec![entry("heatmap"), entry("orgs")];
        store
            .put(LayoutSurface::Profile, "alice", Some(&next))
            .await
            .unwrap();
        assert_eq!(
            store.get(LayoutSurface::Profile, "alice").await.unwrap(),
            Some(next)
        );
    }

    #[tokio::test]
    async fn surfaces_are_independent() {
        let store = MemoryProfileLayoutStore::default();
        let profile = vec![entry("sessions")];
        let home = vec![entry("heatmap"), entry("travel")];

        store
            .put(LayoutSurface::Profile, "alice", Some(&profile))
            .await
            .unwrap();
        store
            .put(LayoutSurface::Home, "alice", Some(&home))
            .await
            .unwrap();

        // Writing one surface must not bleed into the other.
        assert_eq!(
            store.get(LayoutSurface::Profile, "alice").await.unwrap(),
            Some(profile),
        );
        assert_eq!(
            store.get(LayoutSurface::Home, "alice").await.unwrap(),
            Some(home),
        );
    }
}
