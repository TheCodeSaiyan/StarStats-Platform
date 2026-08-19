//! Per-user share metadata — expiry timestamps + human notes.
//!
//! SpiceDB is the source of truth for whether a share exists. This
//! module owns the small Postgres side-table that pairs each
//! (owner_handle, recipient_handle) with optional `expires_at` and
//! `note` fields, so callers can:
//!
//!  * surface "expires in 7 days" / "for the dev cell" annotations
//!    in the UI without inventing schema on top of SpiceDB,
//!  * enforce expiry at read-time in the friend_* handlers (4xx +
//!    lazy SpiceDB row deletion when `expires_at < now()`).
//!
//! The table is keyed on lower-cased handle pairs so the (case-
//! preserved) display handles in SpiceDB collapse to a single row.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::BTreeMap;

/// One metadata row. `expires_at`, `note`, and `scope` are nullable
/// to model "share exists, no extras". `scope = None` means "full
/// manifest" — the legacy behaviour preserved by migration 0025.
#[derive(Debug, Clone)]
pub struct ShareMeta {
    pub owner_handle: String,
    pub recipient_handle: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub note: Option<String>,
    /// Per-share scope clamp (see `sharing_routes::ShareScope` for the
    /// validated shape). Stored untyped at this layer so the store
    /// stays migration-friendly — the HTTP handler is the validator.
    pub scope: Option<Value>,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum ShareMetaError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Maximum length of the optional note. Validation lives in the
/// HTTP handler before this trait is ever called.
pub const NOTE_MAX_LEN: usize = 280;

/// One (handle, count) row returned by [`ShareMetadataStore::top_active_granters`].
/// "Active" = `expires_at IS NULL OR expires_at > NOW()`.
#[derive(Debug, Clone)]
pub struct GranterCount {
    pub handle: String,
    pub active_share_count: i64,
}

/// Aggregated active-share counters surfaced by
/// [`ShareMetadataStore::active_share_counts`]. Drives the admin
/// `/v1/admin/sharing/overview` headline cards.
#[derive(Debug, Clone, Default)]
pub struct ActiveShareCounts {
    /// COUNT(*) of `share_metadata` rows where `expires_at IS NULL OR
    /// expires_at > NOW()`.
    pub total: i64,
    /// Subset of `total` where `expires_at IS NOT NULL AND > NOW()`.
    pub with_expiry: i64,
}

/// Distribution of `share_metadata.scope->>'kind'` across active
/// shares. `NULL` scope counts as `full` (the legacy default preserved
/// by migration 0025).
#[derive(Debug, Clone, Default)]
pub struct ScopeHistogramCounts {
    pub full: i64,
    pub timeline: i64,
    pub aggregates: i64,
    pub tabs: i64,
    /// Per-tab usage tally drawn from the `scope->'tabs'` array on
    /// active rows where `kind = 'tabs'`. Keyed on the tab string;
    /// value = number of active shares listing that tab.
    pub tab_usage: BTreeMap<String, i64>,
}

#[async_trait]
pub trait ShareMetadataStore: Send + Sync + 'static {
    /// Upsert a metadata row. Idempotent — re-granting a share
    /// overwrites the previous expiry/note/scope. Pass `scope = None`
    /// to mean "full manifest" (the legacy default, preserved by
    /// migration 0025).
    async fn upsert(
        &self,
        owner_handle: &str,
        recipient_handle: &str,
        expires_at: Option<DateTime<Utc>>,
        note: Option<&str>,
        scope: Option<&Value>,
    ) -> Result<(), ShareMetaError>;

    /// Look up a single metadata row. `Ok(None)` = "share exists,
    /// no metadata recorded" which the handler treats as no expiry
    /// and no note.
    async fn find(
        &self,
        owner_handle: &str,
        recipient_handle: &str,
    ) -> Result<Option<ShareMeta>, ShareMetaError>;

    /// Bulk lookup for one owner — pairs with `list_shares`.
    async fn list_by_owner(&self, owner_handle: &str) -> Result<Vec<ShareMeta>, ShareMetaError>;

    /// Bulk lookup for one recipient — pairs with
    /// `list_shared_with_me`.
    async fn list_by_recipient(
        &self,
        recipient_handle: &str,
    ) -> Result<Vec<ShareMeta>, ShareMetaError>;

    /// Delete the metadata row when its SpiceDB relation is
    /// revoked. Idempotent — "no row" is a successful outcome.
    async fn delete(
        &self,
        owner_handle: &str,
        recipient_handle: &str,
    ) -> Result<(), ShareMetaError>;

    /// Headline counters for the admin sharing-overview card.
    /// `expires_at IS NULL` counts as active because legacy rows
    /// (pre-migration-0023 expiry) have no expiry and represent
    /// "share until manually revoked".
    async fn active_share_counts(&self) -> Result<ActiveShareCounts, ShareMetaError>;

    /// Top `n` owner handles by active-share count. Ordered by count
    /// DESC, then handle ASC (stable tie-breaker). "Active" matches
    /// [`Self::active_share_counts`].
    async fn top_active_granters(&self, limit: i64) -> Result<Vec<GranterCount>, ShareMetaError>;

    /// Distribution of `scope->>'kind'` across active shares.
    /// NULL scope counts under `full` (legacy default). For
    /// `kind = 'tabs'` rows, also tallies per-tab usage off
    /// `scope->'tabs'`.
    async fn scope_histogram_active(&self) -> Result<ScopeHistogramCounts, ShareMetaError>;
}

pub struct PostgresShareMetadataStore {
    pool: PgPool,
}

impl PostgresShareMetadataStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ShareMetadataStore for PostgresShareMetadataStore {
    async fn upsert(
        &self,
        owner_handle: &str,
        recipient_handle: &str,
        expires_at: Option<DateTime<Utc>>,
        note: Option<&str>,
        scope: Option<&Value>,
    ) -> Result<(), ShareMetaError> {
        // ON CONFLICT mirrors the expression-based PK declaration.
        // `scope` is bound as Option<Value> so a `None` lands as SQL
        // NULL (= "full manifest", the legacy default).
        sqlx::query(
            r#"
            INSERT INTO share_metadata
                (owner_handle, recipient_handle, expires_at, note, scope)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (lower(owner_handle), lower(recipient_handle))
            DO UPDATE SET
                expires_at = EXCLUDED.expires_at,
                note       = EXCLUDED.note,
                scope      = EXCLUDED.scope
            "#,
        )
        .bind(owner_handle)
        .bind(recipient_handle)
        .bind(expires_at)
        .bind(note)
        .bind(scope)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find(
        &self,
        owner_handle: &str,
        recipient_handle: &str,
    ) -> Result<Option<ShareMeta>, ShareMetaError> {
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<DateTime<Utc>>,
                Option<String>,
                Option<Value>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT owner_handle, recipient_handle, expires_at, note, scope, created_at
            FROM share_metadata
            WHERE lower(owner_handle) = lower($1)
              AND lower(recipient_handle) = lower($2)
            "#,
        )
        .bind(owner_handle)
        .bind(recipient_handle)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(o, r, e, n, s, c)| ShareMeta {
            owner_handle: o,
            recipient_handle: r,
            expires_at: e,
            note: n,
            scope: s,
            created_at: c,
        }))
    }

    async fn list_by_owner(&self, owner_handle: &str) -> Result<Vec<ShareMeta>, ShareMetaError> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<DateTime<Utc>>,
                Option<String>,
                Option<Value>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT owner_handle, recipient_handle, expires_at, note, scope, created_at
            FROM share_metadata
            WHERE lower(owner_handle) = lower($1)
            "#,
        )
        .bind(owner_handle)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(o, r, e, n, s, c)| ShareMeta {
                owner_handle: o,
                recipient_handle: r,
                expires_at: e,
                note: n,
                scope: s,
                created_at: c,
            })
            .collect())
    }

    async fn list_by_recipient(
        &self,
        recipient_handle: &str,
    ) -> Result<Vec<ShareMeta>, ShareMetaError> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<DateTime<Utc>>,
                Option<String>,
                Option<Value>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT owner_handle, recipient_handle, expires_at, note, scope, created_at
            FROM share_metadata
            WHERE lower(recipient_handle) = lower($1)
            "#,
        )
        .bind(recipient_handle)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(o, r, e, n, s, c)| ShareMeta {
                owner_handle: o,
                recipient_handle: r,
                expires_at: e,
                note: n,
                scope: s,
                created_at: c,
            })
            .collect())
    }

    async fn delete(
        &self,
        owner_handle: &str,
        recipient_handle: &str,
    ) -> Result<(), ShareMetaError> {
        sqlx::query(
            r#"
            DELETE FROM share_metadata
            WHERE lower(owner_handle) = lower($1)
              AND lower(recipient_handle) = lower($2)
            "#,
        )
        .bind(owner_handle)
        .bind(recipient_handle)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn active_share_counts(&self) -> Result<ActiveShareCounts, ShareMetaError> {
        // Single round-trip: two filtered COUNT()s. `with_expiry` is
        // a strict subset of `total` so we can't sum afterwards.
        let row: (i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) FILTER (WHERE expires_at IS NULL OR expires_at > NOW()) AS total,
                COUNT(*) FILTER (WHERE expires_at IS NOT NULL AND expires_at > NOW()) AS with_expiry
            FROM share_metadata
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(ActiveShareCounts {
            total: row.0,
            with_expiry: row.1,
        })
    }

    async fn top_active_granters(&self, limit: i64) -> Result<Vec<GranterCount>, ShareMetaError> {
        let limit = limit.clamp(1, 200);
        // Group by lower(owner_handle) so the case-preserved display
        // copy in different rows collapses to one bucket; pick the
        // MIN(owner_handle) as a stable representative.
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT
                MIN(owner_handle) AS handle,
                COUNT(*)          AS active_share_count
            FROM share_metadata
            WHERE expires_at IS NULL OR expires_at > NOW()
            GROUP BY lower(owner_handle)
            ORDER BY active_share_count DESC, handle ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(handle, active_share_count)| GranterCount {
                handle,
                active_share_count,
            })
            .collect())
    }

    async fn scope_histogram_active(&self) -> Result<ScopeHistogramCounts, ShareMetaError> {
        // Kind buckets: NULL scope folds into `full`, anything else
        // reads `scope->>'kind'`. One query for the four buckets.
        let kind_rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT
                COALESCE(scope->>'kind', 'full') AS kind,
                COUNT(*)                         AS n
            FROM share_metadata
            WHERE expires_at IS NULL OR expires_at > NOW()
            GROUP BY COALESCE(scope->>'kind', 'full')
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = ScopeHistogramCounts::default();
        for (kind, n) in kind_rows {
            match kind.as_str() {
                "full" => out.full = n,
                "timeline" => out.timeline = n,
                "aggregates" => out.aggregates = n,
                "tabs" => out.tabs = n,
                // Forward-compat: unknown `kind` values surfaced in
                // future migrations land in neither bucket. Logged at
                // debug so we notice if the producer drifts.
                other => tracing::debug!(kind = %other, "unrecognised share scope kind"),
            }
        }
        // Tab-usage tally: explode the `scope->'tabs'` array on active
        // tabs-scope rows. Guard with `jsonb_typeof` so malformed rows
        // don't crash `jsonb_array_elements_text`.
        let tab_rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT tab AS name, COUNT(*) AS n
            FROM share_metadata,
                 LATERAL jsonb_array_elements_text(scope->'tabs') AS tab
            WHERE (expires_at IS NULL OR expires_at > NOW())
              AND scope->>'kind' = 'tabs'
              AND jsonb_typeof(scope->'tabs') = 'array'
            GROUP BY tab
            ORDER BY tab ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        for (name, n) in tab_rows {
            out.tab_usage.insert(name, n);
        }
        Ok(out)
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory store used by the sharing-route tests. Key shape
    /// mirrors the Postgres PK: lower-cased handle pair.
    pub struct MemoryShareMetadataStore {
        rows: Mutex<HashMap<(String, String), ShareMeta>>,
    }

    impl Default for MemoryShareMetadataStore {
        fn default() -> Self {
            Self {
                rows: Mutex::new(HashMap::new()),
            }
        }
    }

    fn key(owner: &str, recipient: &str) -> (String, String) {
        (owner.to_ascii_lowercase(), recipient.to_ascii_lowercase())
    }

    #[async_trait]
    impl ShareMetadataStore for MemoryShareMetadataStore {
        async fn upsert(
            &self,
            owner_handle: &str,
            recipient_handle: &str,
            expires_at: Option<DateTime<Utc>>,
            note: Option<&str>,
            scope: Option<&Value>,
        ) -> Result<(), ShareMetaError> {
            let mut rows = self.rows.lock().unwrap();
            rows.insert(
                key(owner_handle, recipient_handle),
                ShareMeta {
                    owner_handle: owner_handle.to_string(),
                    recipient_handle: recipient_handle.to_string(),
                    expires_at,
                    note: note.map(|s| s.to_string()),
                    scope: scope.cloned(),
                    created_at: Utc::now(),
                },
            );
            Ok(())
        }

        async fn find(
            &self,
            owner_handle: &str,
            recipient_handle: &str,
        ) -> Result<Option<ShareMeta>, ShareMetaError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows.get(&key(owner_handle, recipient_handle)).cloned())
        }

        async fn list_by_owner(
            &self,
            owner_handle: &str,
        ) -> Result<Vec<ShareMeta>, ShareMetaError> {
            let owner_lower = owner_handle.to_ascii_lowercase();
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .iter()
                .filter(|((o, _), _)| o == &owner_lower)
                .map(|(_, m)| m.clone())
                .collect())
        }

        async fn list_by_recipient(
            &self,
            recipient_handle: &str,
        ) -> Result<Vec<ShareMeta>, ShareMetaError> {
            let rec_lower = recipient_handle.to_ascii_lowercase();
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .iter()
                .filter(|((_, r), _)| r == &rec_lower)
                .map(|(_, m)| m.clone())
                .collect())
        }

        async fn delete(
            &self,
            owner_handle: &str,
            recipient_handle: &str,
        ) -> Result<(), ShareMetaError> {
            let mut rows = self.rows.lock().unwrap();
            rows.remove(&key(owner_handle, recipient_handle));
            Ok(())
        }

        async fn active_share_counts(&self) -> Result<ActiveShareCounts, ShareMetaError> {
            let rows = self.rows.lock().unwrap();
            let now = Utc::now();
            let mut total = 0i64;
            let mut with_expiry = 0i64;
            for m in rows.values() {
                let is_active = match m.expires_at {
                    None => true,
                    Some(t) => t > now,
                };
                if is_active {
                    total += 1;
                    if m.expires_at.is_some() {
                        with_expiry += 1;
                    }
                }
            }
            Ok(ActiveShareCounts { total, with_expiry })
        }

        async fn top_active_granters(
            &self,
            limit: i64,
        ) -> Result<Vec<GranterCount>, ShareMetaError> {
            let limit = limit.clamp(1, 200) as usize;
            let rows = self.rows.lock().unwrap();
            let now = Utc::now();
            // Bucket by lower(owner_handle), keeping MIN(owner_handle) as the
            // representative to mirror the Postgres `SELECT MIN(owner_handle)`.
            // This MUST be a deterministic reduction, not the first row seen:
            // `rows` is a HashMap, so iteration order is randomized per process
            // — the old "first-seen" copy made this test flaky across
            // processes/bins (it picked "Alice" or "alice" at random). Rust
            // String Ord is byte order, matching Postgres `MIN` under C
            // collation ("Alice" < "alice").
            let mut counts: HashMap<String, (String, i64)> = HashMap::new();
            for m in rows.values() {
                let active = m.expires_at.map_or(true, |t| t > now);
                if !active {
                    continue;
                }
                let lower = m.owner_handle.to_ascii_lowercase();
                let entry = counts
                    .entry(lower)
                    .or_insert_with(|| (m.owner_handle.clone(), 0));
                if m.owner_handle < entry.0 {
                    entry.0 = m.owner_handle.clone();
                }
                entry.1 += 1;
            }
            let mut ranked: Vec<GranterCount> = counts
                .into_values()
                .map(|(handle, c)| GranterCount {
                    handle,
                    active_share_count: c,
                })
                .collect();
            ranked.sort_by(|a, b| {
                b.active_share_count
                    .cmp(&a.active_share_count)
                    .then_with(|| a.handle.cmp(&b.handle))
            });
            ranked.truncate(limit);
            Ok(ranked)
        }

        async fn scope_histogram_active(&self) -> Result<ScopeHistogramCounts, ShareMetaError> {
            let rows = self.rows.lock().unwrap();
            let now = Utc::now();
            let mut out = ScopeHistogramCounts::default();
            for m in rows.values() {
                let active = m.expires_at.map_or(true, |t| t > now);
                if !active {
                    continue;
                }
                let kind = m
                    .scope
                    .as_ref()
                    .and_then(|s| s.get("kind"))
                    .and_then(Value::as_str)
                    .unwrap_or("full");
                match kind {
                    "full" => out.full += 1,
                    "timeline" => out.timeline += 1,
                    "aggregates" => out.aggregates += 1,
                    "tabs" => {
                        out.tabs += 1;
                        if let Some(arr) = m
                            .scope
                            .as_ref()
                            .and_then(|s| s.get("tabs"))
                            .and_then(Value::as_array)
                        {
                            for v in arr {
                                if let Some(t) = v.as_str() {
                                    *out.tab_usage.entry(t.to_string()).or_insert(0) += 1;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryShareMetadataStore;
    use super::*;
    use serde_json::json;

    fn past() -> DateTime<Utc> {
        Utc::now() - chrono::Duration::hours(1)
    }
    fn future() -> DateTime<Utc> {
        Utc::now() + chrono::Duration::hours(1)
    }

    #[tokio::test]
    async fn upsert_find_round_trips_scope_and_note() {
        let store = MemoryShareMetadataStore::default();
        let scope = json!({ "kind": "timeline" });
        store
            .upsert("Alice", "Bob", Some(future()), Some("hi"), Some(&scope))
            .await
            .unwrap();
        let found = store.find("alice", "bob").await.unwrap().unwrap();
        assert_eq!(found.note.as_deref(), Some("hi"));
        assert_eq!(found.scope, Some(scope));
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_case_insensitive() {
        let store = MemoryShareMetadataStore::default();
        store
            .upsert("Alice", "Bob", None, None, None)
            .await
            .unwrap();
        // Re-grant with different case + a note overwrites, not duplicates.
        store
            .upsert("alice", "BOB", None, Some("note2"), None)
            .await
            .unwrap();
        let owned = store.list_by_owner("ALICE").await.unwrap();
        assert_eq!(owned.len(), 1, "re-grant overwrites the single row");
        assert_eq!(owned[0].note.as_deref(), Some("note2"));
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let store = MemoryShareMetadataStore::default();
        store.upsert("a", "b", None, None, None).await.unwrap();
        store.delete("A", "B").await.unwrap();
        assert!(store.find("a", "b").await.unwrap().is_none());
        // Deleting a non-existent pair is a successful no-op.
        store.delete("a", "b").await.unwrap();
    }

    #[tokio::test]
    async fn active_counts_exclude_expired_and_track_expiry_subset() {
        let store = MemoryShareMetadataStore::default();
        store.upsert("a", "b", None, None, None).await.unwrap(); // active, no expiry
        store
            .upsert("a", "c", Some(future()), None, None)
            .await
            .unwrap(); // active, with expiry
        store
            .upsert("a", "d", Some(past()), None, None)
            .await
            .unwrap(); // expired -> excluded
        let counts = store.active_share_counts().await.unwrap();
        assert_eq!(counts.total, 2, "expired share is not active");
        assert_eq!(counts.with_expiry, 1, "only the future-expiry share counts");
    }

    #[tokio::test]
    async fn histogram_buckets_null_scope_as_full() {
        let store = MemoryShareMetadataStore::default();
        store.upsert("a", "b", None, None, None).await.unwrap(); // NULL -> full
        let scope_no_kind = json!({ "note": "x" });
        store
            .upsert("a", "c", None, None, Some(&scope_no_kind))
            .await
            .unwrap(); // no kind -> full
        let agg = json!({ "kind": "aggregates" });
        store
            .upsert("a", "d", None, None, Some(&agg))
            .await
            .unwrap();
        let h = store.scope_histogram_active().await.unwrap();
        assert_eq!(h.full, 2, "NULL and kind-less scopes both bucket as full");
        assert_eq!(h.aggregates, 1);
        assert_eq!(h.timeline, 0);
    }

    #[tokio::test]
    async fn histogram_tallies_tab_usage_for_tabs_scope() {
        let store = MemoryShareMetadataStore::default();
        let s1 = json!({ "kind": "tabs", "tabs": ["combat", "economy"] });
        let s2 = json!({ "kind": "tabs", "tabs": ["combat"] });
        store.upsert("a", "b", None, None, Some(&s1)).await.unwrap();
        store.upsert("a", "c", None, None, Some(&s2)).await.unwrap();
        let h = store.scope_histogram_active().await.unwrap();
        assert_eq!(h.tabs, 2);
        assert_eq!(h.tab_usage.get("combat"), Some(&2));
        assert_eq!(h.tab_usage.get("economy"), Some(&1));
    }

    #[tokio::test]
    async fn histogram_ignores_expired_rows() {
        let store = MemoryShareMetadataStore::default();
        let tl = json!({ "kind": "timeline" });
        store
            .upsert("a", "b", Some(past()), None, Some(&tl))
            .await
            .unwrap();
        let h = store.scope_histogram_active().await.unwrap();
        assert_eq!(h.timeline, 0, "expired share excluded from the histogram");
    }

    #[tokio::test]
    async fn top_granters_group_by_owner_ordered_by_count() {
        let store = MemoryShareMetadataStore::default();
        // Alice: 2 active (case-variant owner collapses to one bucket).
        store.upsert("Alice", "b", None, None, None).await.unwrap();
        store.upsert("alice", "c", None, None, None).await.unwrap();
        // Bob: 1 active + 1 expired (expired excluded).
        store.upsert("Bob", "d", None, None, None).await.unwrap();
        store
            .upsert("Bob", "e", Some(past()), None, None)
            .await
            .unwrap();
        let top = store.top_active_granters(10).await.unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].handle, "Alice");
        assert_eq!(top[0].active_share_count, 2);
        assert_eq!(top[1].handle, "Bob");
        assert_eq!(top[1].active_share_count, 1);
    }
}
