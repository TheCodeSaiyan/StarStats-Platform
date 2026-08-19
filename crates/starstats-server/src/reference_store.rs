//! Persistent store for the Star Citizen class-name reference catalogue.
//!
//! The store is keyed on `(category, class_name)` — the internal Star
//! Citizen identifier embedded in event payloads, scoped to the kind
//! of entity it refers to (vehicle, weapon, item, location). The daily
//! refresh job pulls each category via the upstream wiki API and upserts
//! every entry; render paths read by `(category, class_name)`
//! case-insensitive to translate raw events into player-friendly metadata.
//!
//! Trait shape: implementers only need the three generic methods
//! (`upsert_entries`, `get_entry`, `list_category`). The legacy
//! vehicle-specific methods are default impls that delegate to the
//! generic ones plus a small ReferenceEntry ↔ VehicleReference
//! conversion — keeps existing callers and tests working through the
//! transition without forcing every implementer to maintain two
//! parallel code paths.
//!
//! Errors collapse into a single [`ReferenceStoreError::Backend`]
//! variant. The route layer treats backend failure as a 503 either
//! way, and there are no unique-constraint races worth carving out a
//! richer taxonomy for. Shape mirrors [`crate::profile_store::ProfileStoreError`].

use crate::reference_data::{ReferenceCategory, ReferenceEntry, VehicleReference};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use starstats_core::location_taxonomy::LocationTaxonomy;

/// Admin dashboard summary for one reference category. `entry_count`
/// is the row count in `reference_registry` filtered by category;
/// `latest_updated_at` is `MAX(updated_at)` (NULL when the category
/// has no rows yet, e.g. a freshly-added one whose cron hasn't run).
#[derive(Debug, Clone)]
pub struct CategorySummary {
    pub category: ReferenceCategory,
    pub entry_count: i64,
    pub latest_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReferenceStoreError {
    #[error("reference store backend error: {0}")]
    Backend(String),
    /// Returned by [`ReferenceStore::reconcile_category`] when the batch
    /// is empty. Refused to prevent a transient wiki outage (which
    /// produces an empty fetch) from clearing the entire category. An
    /// admin path that genuinely wants to empty a category can call
    /// the store directly with a sentinel value or via a future
    /// `clear_category` method.
    #[error("empty batch refused; would clear the category")]
    EmptyBatch,
    /// Returned by [`ReferenceStore::apply_enrichment`] when the
    /// requested metadata namespace key is not a simple lowercase
    /// identifier (`^[a-z_]+$`). Namespaces come from
    /// `enrichment::EnrichmentSource::namespace` static
    /// strings, so this should never fire in practice — it's a
    /// defence-in-depth guard against an injected JSONB path.
    #[error("invalid enrichment namespace: {0}")]
    InvalidNamespace(String),
}

/// Validate that an enrichment namespace is a simple lowercase
/// identifier before it is used as a JSONB path key. Even though the
/// value is bound as a query parameter (never string-interpolated),
/// constraining the shape keeps the metadata object tidy and rejects
/// surprises like empty strings, dots (which would nest), or uppercase.
fn validate_namespace(namespace: &str) -> Result<(), ReferenceStoreError> {
    if !namespace.is_empty()
        && namespace
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        Ok(())
    } else {
        Err(ReferenceStoreError::InvalidNamespace(namespace.to_string()))
    }
}

impl From<sqlx::Error> for ReferenceStoreError {
    fn from(err: sqlx::Error) -> Self {
        Self::Backend(err.to_string())
    }
}

#[async_trait]
pub trait ReferenceStore: Send + Sync + 'static {
    /// Upsert each entry by (category, class_name). Returns the count
    /// of rows affected. Idempotent — repeated calls with the same
    /// payload are cheap and safe.
    ///
    /// Additive: does NOT remove rows missing from `entries`. The
    /// wiki-sync cron uses [`Self::reconcile_category`] instead;
    /// this method is for admin edits and single-row paths that
    /// shouldn't touch the rest of the catalogue.
    async fn upsert_entries(
        &self,
        entries: &[ReferenceEntry],
    ) -> Result<usize, ReferenceStoreError>;

    /// Reconcile a full category against an authoritative external
    /// source: delete rows for `(category, source)` whose `class_name`
    /// is NOT present in `entries`, then upsert every entry in
    /// `entries`. Atomic — the delete + upsert run in a single
    /// transaction so a partial failure rolls back to the prior state.
    ///
    /// All entries MUST have `category == category`; mixed-category
    /// batches are a programming error and the impl is allowed to
    /// behave inconsistently (debug-assert in tests; not enforced at
    /// runtime to avoid the per-row overhead in the hot path).
    ///
    /// Returns the number of rows upserted (matching the
    /// [`Self::upsert_entries`] return shape). Deletion counts are
    /// surfaced via `tracing` inside the impl, not the return value.
    ///
    /// # Errors
    ///
    /// Returns [`ReferenceStoreError::EmptyBatch`] when `entries` is
    /// empty — the wiki returning zero rows for a category is almost
    /// always a transient upstream issue (404 on a renamed page,
    /// schema change, intermittent 502), and silently wiping the
    /// catalogue in that case would surface to users as "everything
    /// disappeared from the KB." Callers that genuinely want to
    /// empty a category should take an explicit admin path.
    ///
    /// # Source scoping
    ///
    /// Only rows with the matching `source` are eligible for deletion.
    /// Today only `"wiki_api"` exists, but the column is the trait's
    /// forward-compat against multi-source ingestion (a future
    /// `community_supplements` source wouldn't be touched by the
    /// wiki-api reconciler).
    async fn reconcile_category(
        &self,
        category: ReferenceCategory,
        source: &str,
        entries: &[ReferenceEntry],
    ) -> Result<usize, ReferenceStoreError>;

    /// Look up a single entry by (category, class_name). Case-insensitive
    /// on `class_name` — game logs occasionally vary case on the same
    /// class.
    async fn get_entry(
        &self,
        category: ReferenceCategory,
        class_name: &str,
    ) -> Result<Option<ReferenceEntry>, ReferenceStoreError>;

    /// Batch-resolve many class names across `categories` (priority order:
    /// earliest category wins) in as few queries as possible. Returns, per
    /// input class name that matched, `(request_class_name, entry)` — the
    /// request string is preserved so the caller keys results by what it sent.
    /// The default loops `get_entry` (Memory store/tests); `PostgresReferenceStore`
    /// overrides with a single `lower(class_name) = ANY(...)` query, replacing
    /// the per-class N-query loop in `reference_resolve`.
    async fn resolve_entries(
        &self,
        categories: &[ReferenceCategory],
        class_names: &[String],
    ) -> Result<Vec<(String, ReferenceEntry)>, ReferenceStoreError> {
        let mut out = Vec::new();
        for name in class_names {
            for &cat in categories {
                if let Some(e) = self.get_entry(cat, name).await? {
                    out.push((name.clone(), e));
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Full list for a category, ordered by `class_name` ASC.
    async fn list_category(
        &self,
        category: ReferenceCategory,
    ) -> Result<Vec<ReferenceEntry>, ReferenceStoreError>;

    /// Apply enrichment taxonomy to existing location rows.
    /// UPDATE-only — never INSERTs. The primary catalogue
    /// (`api.star-citizen.wiki`) is authoritative on row existence;
    /// the enrichment source (`starcitizen.tools`) merely paints
    /// extra fields onto rows that already exist.
    ///
    /// Match is case-insensitive on `slug` AND scoped to
    /// `category = 'location'` — a deliberately narrow surface so
    /// an enrichment payload can't accidentally smear taxonomy
    /// fields onto a vehicle/weapon/item row that happens to share
    /// a slug.
    ///
    /// Returns the count of rows whose enrichment columns were
    /// updated. A slug with no matching primary entry contributes
    /// zero and is logged by the caller (the daily cron) for
    /// human review.
    async fn apply_location_taxonomies(
        &self,
        items: &[(String, LocationTaxonomy)],
    ) -> Result<usize, ReferenceStoreError>;

    /// Generic enrichment primitive (the seam behind the
    /// `enrichment::EnrichmentSource` trait). UPDATE-only — never
    /// INSERTs. For each `(class_name, blob)` pair, merge `blob` into
    /// the matching row's metadata under `namespace` via
    /// `jsonb_set(metadata, ARRAY[namespace], blob, true)`, scoped to
    /// `category` and matched case-insensitively on `class_name`.
    ///
    /// Sibling metadata keys (e.g. the wiki `manufacturer`/`role` or a
    /// `taxonomy_v2` blob) are preserved — only `namespace` is written.
    ///
    /// `namespace` MUST be a simple lowercase identifier (`^[a-z_]+$`)
    /// or this returns [`ReferenceStoreError::InvalidNamespace`].
    /// An empty `pairs` slice returns [`ReferenceStoreError::EmptyBatch`]
    /// — a transient upstream outage (zero matches) must never wipe a
    /// populated namespace.
    ///
    /// Returns the count of rows actually updated; a `class_name` with
    /// no matching row contributes zero and is the caller's to log.
    async fn apply_enrichment(
        &self,
        category: ReferenceCategory,
        namespace: &str,
        pairs: &[(String, serde_json::Value)],
    ) -> Result<usize, ReferenceStoreError>;

    /// Look up a single entry by (category, slug). Case-insensitive
    /// on `slug` for symmetry with `get_entry`. Returns `None` for
    /// rows whose slug column is still null from before the KB
    /// rollout — the route layer falls back to `get_entry` (by
    /// class_name) in that case so legacy URLs keep resolving.
    ///
    /// Default impl walks `list_category` — fine for the in-memory
    /// test store; the Postgres impl overrides with an indexed
    /// `WHERE lower(slug) = lower($2)` query.
    async fn get_by_slug(
        &self,
        category: ReferenceCategory,
        slug: &str,
    ) -> Result<Option<ReferenceEntry>, ReferenceStoreError> {
        let needle = slug.to_lowercase();
        let entries = self.list_category(category).await?;
        Ok(entries
            .into_iter()
            .find(|e| e.slug.as_deref().map(str::to_lowercase) == Some(needle.clone())))
    }

    /// Admin-only: row counts + latest update timestamp per category.
    /// Cheap aggregate over the registry; the admin dashboard uses
    /// this to surface stale categories at a glance.
    async fn category_summaries(&self) -> Result<Vec<CategorySummary>, ReferenceStoreError> {
        // Default impl: walk each category via list_category. Slow
        // but correct for the in-memory test store; the Postgres
        // impl overrides with a single GROUP BY aggregate.
        let mut out = Vec::new();
        for c in [
            ReferenceCategory::Vehicle,
            ReferenceCategory::Weapon,
            ReferenceCategory::Item,
            ReferenceCategory::Location,
        ] {
            let entries = self.list_category(c).await?;
            out.push(CategorySummary {
                category: c,
                entry_count: entries.len() as i64,
                latest_updated_at: None,
            });
        }
        Ok(out)
    }

    // -- Legacy vehicle-shaped helpers ---------------------------------
    //
    // Default impls delegate to the generic methods + per-category
    // conversion. Implementers don't need to override these. The
    // in-tree cron no longer calls `upsert_vehicles` after P3, but
    // the method stays on the trait for backwards compatibility with
    // external implementers and the existing tests.

    #[allow(dead_code)]
    async fn upsert_vehicles(
        &self,
        vehicles: &[VehicleReference],
    ) -> Result<usize, ReferenceStoreError> {
        let entries: Vec<ReferenceEntry> = vehicles.iter().map(vehicle_to_entry).collect();
        self.upsert_entries(&entries).await
    }

    async fn get_vehicle(
        &self,
        class_name: &str,
    ) -> Result<Option<VehicleReference>, ReferenceStoreError> {
        Ok(self
            .get_entry(ReferenceCategory::Vehicle, class_name)
            .await?
            .map(entry_to_vehicle))
    }

    async fn list_vehicles(&self) -> Result<Vec<VehicleReference>, ReferenceStoreError> {
        Ok(self
            .list_category(ReferenceCategory::Vehicle)
            .await?
            .into_iter()
            .map(entry_to_vehicle)
            .collect())
    }
}

/// `ReferenceEntry` (generic) → typed `VehicleReference` view. Used by
/// the legacy `get_vehicle` / `list_vehicles` path.
pub(crate) fn entry_to_vehicle(e: ReferenceEntry) -> VehicleReference {
    let meta = e.metadata.as_object().cloned().unwrap_or_default();
    let s = |k: &str| {
        meta.get(k)
            .and_then(serde_json::Value::as_str)
            .map(String::from)
    };
    VehicleReference {
        class_name: e.class_name,
        display_name: e.display_name,
        manufacturer: s("manufacturer"),
        role: s("role"),
        hull_size: s("hull_size"),
        focus: s("focus"),
    }
}

/// Typed `VehicleReference` → generic `ReferenceEntry`. Collapses the
/// typed columns into a metadata JSON object, dropping `None` fields
/// so they don't pollute the JSONB body with explicit `null`s.
#[allow(dead_code)]
pub(crate) fn vehicle_to_entry(v: &VehicleReference) -> ReferenceEntry {
    let mut meta = serde_json::Map::new();
    let mut put = |k: &str, val: &Option<String>| {
        if let Some(s) = val {
            meta.insert(k.into(), serde_json::Value::String(s.clone()));
        }
    };
    put("manufacturer", &v.manufacturer);
    put("role", &v.role);
    put("hull_size", &v.hull_size);
    put("focus", &v.focus);
    ReferenceEntry {
        category: ReferenceCategory::Vehicle,
        class_name: v.class_name.clone(),
        display_name: v.display_name.clone(),
        // Legacy vehicle conversion predates the KB slug rollout —
        // callers that still ingest via this path will get a slug
        // assigned the next time the wiki sync runs.
        slug: None,
        metadata: serde_json::Value::Object(meta),
    }
}

// -- Postgres impl ---------------------------------------------------
//
// `updated_at` and `source` are intentionally omitted from the
// surfaced `SELECT` columns. `updated_at` drives ops alerting
// (stale-cache detection); `source` is provenance for debugging the
// refresh path. Neither is part of the public response shape.

pub struct PostgresReferenceStore {
    pool: PgPool,
}

impl PostgresReferenceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReferenceStore for PostgresReferenceStore {
    async fn upsert_entries(
        &self,
        entries: &[ReferenceEntry],
    ) -> Result<usize, ReferenceStoreError> {
        // Wrap the batch in a transaction so a partial failure (e.g. a
        // constraint violation midway) rolls back cleanly. The caller
        // treats "fresh refresh" as all-or-nothing — partial writes
        // would leave the cache in an inconsistent state where some
        // rows point at last-week's metadata and others at today's.
        let mut tx = self.pool.begin().await?;
        let mut affected: u64 = 0;

        for e in entries {
            // `slug` uses `COALESCE(EXCLUDED.slug, reference_registry.slug)`
            // on conflict so a caller that passes `slug = None`
            // doesn't accidentally wipe a previously-backfilled
            // slug. Under normal operation the wiki-sync cron
            // computes slugs via `apply_slug_collisions` and always
            // passes `Some(_)`, so EXCLUDED wins. The COALESCE
            // covers the legacy `vehicle_to_entry` path (which
            // sets `slug: None` for backwards compat) — it would
            // otherwise null out backfilled slugs from the
            // migration's location backfill.
            let result = sqlx::query(
                r#"
                INSERT INTO reference_registry
                    (category, class_name, display_name, slug, metadata, source, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, NOW())
                ON CONFLICT (category, class_name) DO UPDATE
                    SET display_name = EXCLUDED.display_name,
                        slug         = COALESCE(EXCLUDED.slug, reference_registry.slug),
                        metadata     = EXCLUDED.metadata,
                        source       = EXCLUDED.source,
                        updated_at   = NOW()
                "#,
            )
            .bind(e.category.as_str())
            .bind(&e.class_name)
            .bind(&e.display_name)
            .bind(&e.slug)
            .bind(&e.metadata)
            .bind("wiki_api")
            .execute(&mut *tx)
            .await?;
            affected = affected.saturating_add(result.rows_affected());
        }

        tx.commit().await?;
        Ok(affected as usize)
    }

    async fn reconcile_category(
        &self,
        category: ReferenceCategory,
        source: &str,
        entries: &[ReferenceEntry],
    ) -> Result<usize, ReferenceStoreError> {
        if entries.is_empty() {
            return Err(ReferenceStoreError::EmptyBatch);
        }

        // Delete stale rows first, then upsert. Both inside a single
        // transaction so a constraint violation on the upsert rolls
        // back the delete — the catalogue is never half-mirror.
        let mut tx = self.pool.begin().await?;

        // class_names present in the new batch. Matched against
        // `class_name` (exact, byte-equal) — the upsert keys on the
        // same column without lower()-folding, so a case-swapped row
        // is treated as "stale + new pair" and gets cleaned up by the
        // delete-stale step. That's the right behavior: the (category,
        // class_name) PK can't be byte-fuzzy without also breaking the
        // ON CONFLICT clause.
        let keep_class_names: Vec<String> = entries.iter().map(|e| e.class_name.clone()).collect();

        let delete_result = sqlx::query(
            r#"
            DELETE FROM reference_registry
             WHERE category = $1
               AND source   = $2
               AND class_name <> ALL($3::TEXT[])
            "#,
        )
        .bind(category.as_str())
        .bind(source)
        .bind(&keep_class_names)
        .execute(&mut *tx)
        .await?;
        let removed = delete_result.rows_affected();

        // Clear slugs for the surviving rows BEFORE the per-row upsert.
        // `apply_slug_collisions` derives `base` / `base-2` / `base-3`
        // suffixes by sort position, so adding or removing a single
        // entry can cascade-shift slugs across many rows (e.g. a new
        // "Seat" pushes an existing "seat" → "seat-2", whose old slug a
        // sibling row still holds). Upserting row-by-row then transiently
        // collides on the partial unique index `(category, lower(slug))
        // WHERE slug IS NOT NULL` the moment a row takes a slug another
        // not-yet-updated row still holds — rolling back the whole
        // reconcile (the item-category 5xx). Nulling first lifts every
        // row out of the partial index, so the re-set (whose batch slugs
        // are already unique) can never collide mid-transaction. On any
        // failure the null rolls back with the rest of the tx.
        sqlx::query(
            r#"
            UPDATE reference_registry
               SET slug = NULL
             WHERE category = $1
               AND source   = $2
               AND slug IS NOT NULL
            "#,
        )
        .bind(category.as_str())
        .bind(source)
        .execute(&mut *tx)
        .await?;

        let mut affected: u64 = 0;
        for e in entries {
            let result = sqlx::query(
                r#"
                INSERT INTO reference_registry
                    (category, class_name, display_name, slug, metadata, source, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, NOW())
                ON CONFLICT (category, class_name) DO UPDATE
                    SET display_name = EXCLUDED.display_name,
                        slug         = COALESCE(EXCLUDED.slug, reference_registry.slug),
                        metadata     = EXCLUDED.metadata,
                        source       = EXCLUDED.source,
                        updated_at   = NOW()
                "#,
            )
            .bind(e.category.as_str())
            .bind(&e.class_name)
            .bind(&e.display_name)
            .bind(&e.slug)
            .bind(&e.metadata)
            .bind(source)
            .execute(&mut *tx)
            .await?;
            affected = affected.saturating_add(result.rows_affected());
        }

        tx.commit().await?;

        tracing::info!(
            category = category.as_str(),
            source = source,
            removed = removed,
            upserted = affected,
            "reference category reconciled"
        );

        Ok(affected as usize)
    }

    async fn get_entry(
        &self,
        category: ReferenceCategory,
        class_name: &str,
    ) -> Result<Option<ReferenceEntry>, ReferenceStoreError> {
        let row: Option<(String, String, Option<String>, serde_json::Value)> = sqlx::query_as(
            "SELECT class_name, display_name, slug, metadata \
                 FROM reference_registry \
                 WHERE category = $1 AND lower(class_name) = lower($2)",
        )
        .bind(category.as_str())
        .bind(class_name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(class_name, display_name, slug, metadata)| ReferenceEntry {
                category,
                class_name,
                display_name,
                slug,
                metadata,
            },
        ))
    }

    async fn resolve_entries(
        &self,
        categories: &[ReferenceCategory],
        class_names: &[String],
    ) -> Result<Vec<(String, ReferenceEntry)>, ReferenceStoreError> {
        if categories.is_empty() || class_names.is_empty() {
            return Ok(Vec::new());
        }
        // One case-insensitive query across every category; precedence + the
        // request→canonical case mapping are applied in Rust.
        let cat_strs: Vec<String> = categories.iter().map(|c| c.as_str().to_string()).collect();
        let lowered: Vec<String> = class_names.iter().map(|c| c.to_lowercase()).collect();
        let rows: Vec<(String, String, String, Option<String>, serde_json::Value)> =
            sqlx::query_as(
                "SELECT category, class_name, display_name, slug, metadata \
                     FROM reference_registry \
                     WHERE category = ANY($1) AND lower(class_name) = ANY($2)",
            )
            .bind(&cat_strs)
            .bind(&lowered)
            .fetch_all(&self.pool)
            .await?;

        // Category → priority index (earlier = wins).
        let prio: std::collections::HashMap<&str, usize> = categories
            .iter()
            .enumerate()
            .map(|(i, c)| (c.as_str(), i))
            .collect();
        // lowercased class_name → the request string that asked for it.
        let req_by_lower: std::collections::HashMap<String, &String> =
            class_names.iter().map(|n| (n.to_lowercase(), n)).collect();
        // Best (lowest-priority-index) match per lowercased class.
        let mut best: std::collections::HashMap<String, (usize, ReferenceEntry)> =
            std::collections::HashMap::new();
        for (cat_s, class_name, display_name, slug, metadata) in rows {
            let Some(cat) = ReferenceCategory::parse(&cat_s) else {
                continue;
            };
            let p = *prio.get(cat_s.as_str()).unwrap_or(&usize::MAX);
            let lower = class_name.to_lowercase();
            if best.get(&lower).map(|(bp, _)| *bp).unwrap_or(usize::MAX) <= p {
                continue;
            }
            best.insert(
                lower,
                (
                    p,
                    ReferenceEntry {
                        category: cat,
                        class_name,
                        display_name,
                        slug,
                        metadata,
                    },
                ),
            );
        }

        Ok(best
            .into_iter()
            .filter_map(|(lower, (_, entry))| {
                req_by_lower.get(&lower).map(|req| ((*req).clone(), entry))
            })
            .collect())
    }

    async fn list_category(
        &self,
        category: ReferenceCategory,
    ) -> Result<Vec<ReferenceEntry>, ReferenceStoreError> {
        let rows: Vec<(String, String, Option<String>, serde_json::Value)> = sqlx::query_as(
            "SELECT class_name, display_name, slug, metadata \
                 FROM reference_registry \
                 WHERE category = $1 \
                 ORDER BY class_name ASC",
        )
        .bind(category.as_str())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(class_name, display_name, slug, metadata)| ReferenceEntry {
                    category,
                    class_name,
                    display_name,
                    slug,
                    metadata,
                },
            )
            .collect())
    }

    /// Postgres impl of the enrichment path. Each item gets its own
    /// UPDATE inside a single transaction — batching via UNNEST is
    /// possible but adds JSONB-array binding gymnastics for a daily
    /// cron whose row-count tops out at ~1000. The simple per-row
    /// loop is fine at this cadence and keeps the SQL readable.
    ///
    /// The UPDATE writes:
    ///   * `tier` and `subtype` columns — indexable for the
    ///     journey-page filter hot path (Phase 3+).
    ///   * `taxonomy_v2` column — the full enrichment JSONB blob.
    ///   * `metadata.taxonomy_v2` — same blob mirrored INTO the
    ///     existing metadata so the `build_summary` read path picks
    ///     it up without needing to know about the new columns.
    ///     Slight duplication, zero coordination cost.
    async fn apply_location_taxonomies(
        &self,
        items: &[(String, LocationTaxonomy)],
    ) -> Result<usize, ReferenceStoreError> {
        let mut tx = self.pool.begin().await?;
        let mut affected: u64 = 0;

        for (slug, taxonomy) in items {
            let tier_str: Option<&'static str> = taxonomy.tier.map(|t| t.as_str());
            let subtype_str: Option<&str> = taxonomy.subtype.as_deref();
            let blob = serde_json::to_value(taxonomy)
                .map_err(|e| ReferenceStoreError::Backend(e.to_string()))?;

            let result = sqlx::query(
                r#"
                UPDATE reference_registry
                   SET tier        = $1,
                       subtype     = $2,
                       taxonomy_v2 = $3,
                       metadata    = jsonb_set(metadata, '{taxonomy_v2}', $3, true),
                       updated_at  = NOW()
                 WHERE category = 'location'
                   AND lower(slug) = lower($4)
                "#,
            )
            .bind(tier_str)
            .bind(subtype_str)
            .bind(&blob)
            .bind(slug)
            .execute(&mut *tx)
            .await?;
            affected = affected.saturating_add(result.rows_affected());
        }

        tx.commit().await?;
        Ok(affected as usize)
    }

    async fn apply_enrichment(
        &self,
        category: ReferenceCategory,
        namespace: &str,
        pairs: &[(String, serde_json::Value)],
    ) -> Result<usize, ReferenceStoreError> {
        if pairs.is_empty() {
            return Err(ReferenceStoreError::EmptyBatch);
        }
        validate_namespace(namespace)?;

        let mut tx = self.pool.begin().await?;
        let mut affected: u64 = 0;

        for (class_name, blob) in pairs {
            // `ARRAY[$2]::text[]` is the JSONB path; $2 is bound, never
            // interpolated, so even without `validate_namespace` there
            // is no injection surface. `true` = create the key if absent.
            let result = sqlx::query(
                r#"
                UPDATE reference_registry
                   SET metadata   = jsonb_set(metadata, ARRAY[$2]::text[], $3, true),
                       updated_at = NOW()
                 WHERE category = $1
                   AND lower(class_name) = lower($4)
                "#,
            )
            .bind(category.as_str())
            .bind(namespace)
            .bind(blob)
            .bind(class_name)
            .execute(&mut *tx)
            .await?;
            affected = affected.saturating_add(result.rows_affected());
        }

        tx.commit().await?;
        Ok(affected as usize)
    }

    async fn get_by_slug(
        &self,
        category: ReferenceCategory,
        slug: &str,
    ) -> Result<Option<ReferenceEntry>, ReferenceStoreError> {
        let row: Option<(String, String, Option<String>, serde_json::Value)> = sqlx::query_as(
            "SELECT class_name, display_name, slug, metadata \
                 FROM reference_registry \
                 WHERE category = $1 AND lower(slug) = lower($2)",
        )
        .bind(category.as_str())
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(class_name, display_name, slug, metadata)| ReferenceEntry {
                category,
                class_name,
                display_name,
                slug,
                metadata,
            },
        ))
    }

    async fn category_summaries(&self) -> Result<Vec<CategorySummary>, ReferenceStoreError> {
        // Single GROUP BY beats 4 list_category round-trips. Outer
        // LEFT JOIN against the static category list keeps every
        // category present even when it has no rows yet, which
        // matters for "is the location sync running?" diagnostics.
        let rows: Vec<(String, i64, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT cat, COALESCE(cnt, 0), latest
             FROM unnest(ARRAY['vehicle','weapon','item','location']) AS cat
             LEFT JOIN (
                 SELECT category, COUNT(*) AS cnt, MAX(updated_at) AS latest
                 FROM reference_registry
                 GROUP BY category
             ) agg ON agg.category = cat",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for (cat_str, count, latest) in rows {
            let Some(category) = ReferenceCategory::parse(&cat_str) else {
                tracing::warn!(category = %cat_str, "unknown reference category in summary");
                continue;
            };
            out.push(CategorySummary {
                category,
                entry_count: count,
                latest_updated_at: latest,
            });
        }
        Ok(out)
    }
}

// -- Test impl + tests -----------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory implementation used by handler-level tests. Mirrors
    /// Postgres semantics: idempotent upsert keyed on
    /// (category, lower(class_name)), case-insensitive lookup,
    /// ASCII-sorted list per category.
    #[derive(Default)]
    pub struct MemoryReferenceStore {
        rows: Mutex<HashMap<(&'static str, String), ReferenceEntry>>,
    }

    impl MemoryReferenceStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl ReferenceStore for MemoryReferenceStore {
        async fn upsert_entries(
            &self,
            entries: &[ReferenceEntry],
        ) -> Result<usize, ReferenceStoreError> {
            let mut rows = self.rows.lock().unwrap();
            let mut affected = 0usize;
            for e in entries {
                let key = (e.category.as_str(), e.class_name.to_lowercase());
                rows.insert(key, e.clone());
                affected = affected.saturating_add(1);
            }
            Ok(affected)
        }

        async fn reconcile_category(
            &self,
            category: ReferenceCategory,
            _source: &str,
            entries: &[ReferenceEntry],
        ) -> Result<usize, ReferenceStoreError> {
            if entries.is_empty() {
                return Err(ReferenceStoreError::EmptyBatch);
            }

            // MemoryReferenceStore doesn't track `source` per row
            // (Postgres does — see column on `reference_registry`).
            // Treat every memory-stored row as `source = "wiki_api"`
            // implicitly; tests that need source isolation should use
            // the Postgres impl. The `_source` param keeps the trait
            // shape but is unused here.
            let mut rows = self.rows.lock().unwrap();

            let keep: std::collections::HashSet<String> =
                entries.iter().map(|e| e.class_name.clone()).collect();
            let cat_str = category.as_str();
            rows.retain(|(cat, _), entry| *cat != cat_str || keep.contains(&entry.class_name));

            let mut affected = 0usize;
            for e in entries {
                let key = (e.category.as_str(), e.class_name.to_lowercase());
                rows.insert(key, e.clone());
                affected = affected.saturating_add(1);
            }
            Ok(affected)
        }

        async fn get_entry(
            &self,
            category: ReferenceCategory,
            class_name: &str,
        ) -> Result<Option<ReferenceEntry>, ReferenceStoreError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .get(&(category.as_str(), class_name.to_lowercase()))
                .cloned())
        }

        async fn list_category(
            &self,
            category: ReferenceCategory,
        ) -> Result<Vec<ReferenceEntry>, ReferenceStoreError> {
            let rows = self.rows.lock().unwrap();
            let mut out: Vec<ReferenceEntry> = rows
                .iter()
                .filter(|((cat, _), _)| *cat == category.as_str())
                .map(|(_, v)| v.clone())
                .collect();
            out.sort_by(|a, b| a.class_name.cmp(&b.class_name));
            Ok(out)
        }

        /// Memory-impl enrichment. Mirrors Postgres semantics:
        /// case-insensitive slug match, scoped to
        /// `category = 'location'`, writes the full taxonomy blob
        /// into `metadata.taxonomy_v2` so route-layer tests can
        /// observe the same shape Postgres would produce.
        async fn apply_location_taxonomies(
            &self,
            items: &[(String, LocationTaxonomy)],
        ) -> Result<usize, ReferenceStoreError> {
            let mut rows = self.rows.lock().unwrap();
            let mut affected = 0usize;

            for (slug, taxonomy) in items {
                // Find a location-category row whose slug matches
                // case-insensitively. The map is keyed on
                // `(category, lower(class_name))` so we have to scan
                // — fine for handler tests that seed <10 rows.
                let needle = slug.to_lowercase();
                let matching_key = rows
                    .iter()
                    .find(|((cat, _), e)| {
                        *cat == "location"
                            && e.slug
                                .as_deref()
                                .map(|s| s.to_lowercase() == needle)
                                .unwrap_or(false)
                    })
                    .map(|(k, _)| k.clone());

                let Some(key) = matching_key else { continue };
                let Some(entry) = rows.get_mut(&key) else {
                    continue;
                };

                let blob = serde_json::to_value(taxonomy)
                    .map_err(|e| ReferenceStoreError::Backend(e.to_string()))?;

                // Mirror the Postgres `jsonb_set(metadata,
                // '{taxonomy_v2}', $3, true)` step: insert (or
                // replace) `taxonomy_v2` on the metadata object.
                // If metadata isn't an object (defensive — the
                // upsert path always writes an object), promote it.
                if !entry.metadata.is_object() {
                    entry.metadata = serde_json::Value::Object(serde_json::Map::new());
                }
                if let Some(obj) = entry.metadata.as_object_mut() {
                    obj.insert("taxonomy_v2".to_string(), blob);
                }
                affected = affected.saturating_add(1);
            }

            Ok(affected)
        }

        async fn apply_enrichment(
            &self,
            category: ReferenceCategory,
            namespace: &str,
            pairs: &[(String, serde_json::Value)],
        ) -> Result<usize, ReferenceStoreError> {
            if pairs.is_empty() {
                return Err(ReferenceStoreError::EmptyBatch);
            }
            validate_namespace(namespace)?;

            let mut rows = self.rows.lock().unwrap();
            let mut affected = 0usize;
            // Map key is `(&'static str, String)` — category as a static
            // str, class_name lowercased.
            let cat: &'static str = category.as_str();

            for (class_name, blob) in pairs {
                // Map is keyed on `(category, lower(class_name))`, so an
                // exact key probe is enough — no scan needed here.
                let key = (cat, class_name.to_lowercase());
                let Some(entry) = rows.get_mut(&key) else {
                    continue;
                };
                // Mirror the Postgres `jsonb_set(metadata,
                // ARRAY[namespace], blob, true)` — set/replace exactly
                // `namespace`, leaving sibling keys untouched.
                if !entry.metadata.is_object() {
                    entry.metadata = serde_json::Value::Object(serde_json::Map::new());
                }
                if let Some(obj) = entry.metadata.as_object_mut() {
                    obj.insert(namespace.to_string(), blob.clone());
                }
                affected = affected.saturating_add(1);
            }

            Ok(affected)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryReferenceStore;
    use super::*;

    fn make_vehicle(class_name: &str, display_name: &str) -> VehicleReference {
        VehicleReference {
            class_name: class_name.to_owned(),
            display_name: display_name.to_owned(),
            manufacturer: Some("Aegis Dynamics".to_owned()),
            role: Some("Heavy Fighter".to_owned()),
            hull_size: Some("Small".to_owned()),
            focus: Some("Combat".to_owned()),
        }
    }

    #[tokio::test]
    async fn upsert_and_get_round_trips() {
        let store = MemoryReferenceStore::new();
        let v = make_vehicle("AEGS_Avenger_Stalker", "Aegis Avenger Stalker");
        let affected = store.upsert_vehicles(&[v.clone()]).await.unwrap();
        assert_eq!(affected, 1);

        let got = store
            .get_vehicle("AEGS_Avenger_Stalker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, v);

        let updated = VehicleReference {
            display_name: "Aegis Avenger Stalker (Refreshed)".to_owned(),
            ..v.clone()
        };
        store.upsert_vehicles(&[updated.clone()]).await.unwrap();
        let got = store
            .get_vehicle("AEGS_Avenger_Stalker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.display_name, "Aegis Avenger Stalker (Refreshed)");

        assert!(store
            .get_vehicle("not-a-real-class")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn get_vehicle_is_case_insensitive() {
        let store = MemoryReferenceStore::new();
        let v = make_vehicle("AEGS_Avenger_Stalker", "Aegis Avenger Stalker");
        store.upsert_vehicles(&[v.clone()]).await.unwrap();

        let lower = store
            .get_vehicle("aegs_avenger_stalker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lower, v);

        let upper = store
            .get_vehicle("AEGS_AVENGER_STALKER")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(upper, v);
    }

    #[tokio::test]
    async fn list_vehicles_orders_by_class_name_ascending() {
        let store = MemoryReferenceStore::new();
        store
            .upsert_vehicles(&[
                make_vehicle("DRAK_Cutlass_Black", "Drake Cutlass Black"),
                make_vehicle("AEGS_Avenger_Stalker", "Aegis Avenger Stalker"),
                make_vehicle("ANVL_Hornet_F7C", "Anvil Hornet F7C"),
            ])
            .await
            .unwrap();

        let listed = store.list_vehicles().await.unwrap();
        let class_names: Vec<&str> = listed.iter().map(|v| v.class_name.as_str()).collect();
        assert_eq!(
            class_names,
            vec![
                "AEGS_Avenger_Stalker",
                "ANVL_Hornet_F7C",
                "DRAK_Cutlass_Black"
            ]
        );
    }

    #[tokio::test]
    async fn generic_entries_are_scoped_by_category() {
        let store = MemoryReferenceStore::new();
        let mut meta = serde_json::Map::new();
        meta.insert(
            "damage_type".into(),
            serde_json::Value::String("Energy".into()),
        );
        let weapon = ReferenceEntry {
            category: ReferenceCategory::Weapon,
            class_name: "KLWE_LaserCannon_S2".to_owned(),
            display_name: "Klaus & Werner Sledge II".to_owned(),
            slug: None,
            metadata: serde_json::Value::Object(meta),
        };
        store.upsert_entries(&[weapon.clone()]).await.unwrap();

        // Same class_name under a different category must not collide.
        let vehicle_with_same_id = ReferenceEntry {
            category: ReferenceCategory::Vehicle,
            class_name: "KLWE_LaserCannon_S2".to_owned(),
            display_name: "(theoretical) some other thing".to_owned(),
            slug: None,
            metadata: serde_json::Value::Object(Default::default()),
        };
        store
            .upsert_entries(&[vehicle_with_same_id.clone()])
            .await
            .unwrap();

        let got_weapon = store
            .get_entry(ReferenceCategory::Weapon, "klwe_lasercannon_s2")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got_weapon.display_name, "Klaus & Werner Sledge II");

        let got_vehicle = store
            .get_entry(ReferenceCategory::Vehicle, "KLWE_LaserCannon_S2")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got_vehicle.display_name, "(theoretical) some other thing");

        // Cross-category lookup returns None.
        assert!(store
            .get_entry(ReferenceCategory::Item, "KLWE_LaserCannon_S2")
            .await
            .unwrap()
            .is_none());
    }

    // ---- apply_location_taxonomies ---------------------------------

    use starstats_core::location_taxonomy::{LocationTier, Placement};

    fn make_location(class_name: &str, slug: &str) -> ReferenceEntry {
        ReferenceEntry {
            category: ReferenceCategory::Location,
            class_name: class_name.to_owned(),
            display_name: class_name.to_owned(),
            slug: Some(slug.to_owned()),
            metadata: serde_json::json!({
                "star":   { "name": "Stanton" },
                "parent": { "name": "Hurston" },
                "tag":    { "name": class_name },
                "type":   { "classification": "Settlement" }
            }),
        }
    }

    #[tokio::test]
    async fn apply_location_taxonomies_writes_metadata_for_matching_slug() {
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[make_location("Lorville", "lorville")])
            .await
            .unwrap();

        let taxonomy = LocationTaxonomy {
            tier: Some(LocationTier::LandingZone),
            subtype: Some("city".to_owned()),
            placement: Some(Placement::OnBody {
                body: "Hurston".to_owned(),
            }),
            operator: Some("Hurston Dynamics".to_owned()),
            faction: None,
            additional_categories: vec![],
        };

        let affected = store
            .apply_location_taxonomies(&[("lorville".to_owned(), taxonomy)])
            .await
            .unwrap();
        assert_eq!(affected, 1);

        let entry = store
            .get_entry(ReferenceCategory::Location, "Lorville")
            .await
            .unwrap()
            .unwrap();
        let blob = entry
            .metadata
            .get("taxonomy_v2")
            .expect("taxonomy_v2 must be written into metadata");
        assert_eq!(
            blob.get("tier").and_then(|v| v.as_str()),
            Some("landing_zone")
        );
        assert_eq!(blob.get("subtype").and_then(|v| v.as_str()), Some("city"));
        assert_eq!(
            blob.pointer("/placement/kind").and_then(|v| v.as_str()),
            Some("on_body")
        );
        assert_eq!(
            blob.pointer("/placement/body").and_then(|v| v.as_str()),
            Some("Hurston")
        );
        assert_eq!(
            blob.get("operator").and_then(|v| v.as_str()),
            Some("Hurston Dynamics")
        );
    }

    #[tokio::test]
    async fn apply_location_taxonomies_is_case_insensitive_on_slug() {
        // Real wiki page titles round-trip to lower-kebab via
        // `slug_from_page_title`, but the enrichment cron might
        // hand us a slug with different casing during a migration.
        // The store should match either way.
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[make_location("Lorville", "lorville")])
            .await
            .unwrap();

        let n = store
            .apply_location_taxonomies(&[(
                "LORVILLE".to_owned(),
                LocationTaxonomy {
                    tier: Some(LocationTier::LandingZone),
                    ..LocationTaxonomy::default()
                },
            )])
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn apply_location_taxonomies_skips_unmatched_slug() {
        let store = MemoryReferenceStore::new();
        let n = store
            .apply_location_taxonomies(&[(
                "no-such-place".to_owned(),
                LocationTaxonomy {
                    tier: Some(LocationTier::Landmark),
                    ..LocationTaxonomy::default()
                },
            )])
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn apply_location_taxonomies_does_not_touch_non_location_rows() {
        let store = MemoryReferenceStore::new();
        // Vehicle with a slug that happens to collide with a
        // location slug — must not be enriched.
        store
            .upsert_entries(&[ReferenceEntry {
                category: ReferenceCategory::Vehicle,
                class_name: "AEGS_Avenger".to_owned(),
                display_name: "Aegis Avenger".to_owned(),
                slug: Some("aegis-avenger".to_owned()),
                metadata: serde_json::json!({}),
            }])
            .await
            .unwrap();

        let n = store
            .apply_location_taxonomies(&[(
                "aegis-avenger".to_owned(),
                LocationTaxonomy {
                    tier: Some(LocationTier::Landmark),
                    ..LocationTaxonomy::default()
                },
            )])
            .await
            .unwrap();
        assert_eq!(n, 0, "vehicle row must not pick up location taxonomy");

        let entry = store
            .get_entry(ReferenceCategory::Vehicle, "AEGS_Avenger")
            .await
            .unwrap()
            .unwrap();
        assert!(entry.metadata.get("taxonomy_v2").is_none());
    }

    // ---- apply_enrichment (generic seam) ---------------------------

    #[tokio::test]
    async fn apply_enrichment_writes_under_namespace_preserving_siblings() {
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[ReferenceEntry {
                category: ReferenceCategory::Vehicle,
                class_name: "AEGS_Avenger_Stalker".to_owned(),
                display_name: "Aegis Avenger Stalker".to_owned(),
                slug: Some("aegis-avenger-stalker".to_owned()),
                // Pre-existing sibling keys that MUST survive.
                metadata: serde_json::json!({ "manufacturer": "Aegis Dynamics", "role": "Fighter" }),
            }])
            .await
            .unwrap();

        let n = store
            .apply_enrichment(
                ReferenceCategory::Vehicle,
                "ship_matrix",
                &[(
                    "AEGS_Avenger_Stalker".to_owned(),
                    serde_json::json!({ "specs": { "max_crew": 1 } }),
                )],
            )
            .await
            .unwrap();
        assert_eq!(n, 1);

        let entry = store
            .get_entry(ReferenceCategory::Vehicle, "AEGS_Avenger_Stalker")
            .await
            .unwrap()
            .unwrap();
        // New namespace landed...
        assert_eq!(
            entry.metadata["ship_matrix"]["specs"]["max_crew"]
                .as_i64()
                .unwrap(),
            1
        );
        // ...and the wiki siblings are untouched.
        assert_eq!(entry.metadata["manufacturer"], "Aegis Dynamics");
        assert_eq!(entry.metadata["role"], "Fighter");
    }

    #[tokio::test]
    async fn apply_enrichment_is_case_insensitive_and_category_scoped() {
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[ReferenceEntry {
                category: ReferenceCategory::Vehicle,
                class_name: "AEGS_Avenger_Stalker".to_owned(),
                display_name: "Aegis Avenger Stalker".to_owned(),
                slug: None,
                metadata: serde_json::json!({}),
            }])
            .await
            .unwrap();

        // Lowercased class_name still matches.
        let n = store
            .apply_enrichment(
                ReferenceCategory::Vehicle,
                "ship_matrix",
                &[(
                    "aegs_avenger_stalker".to_owned(),
                    serde_json::json!({ "ok": true }),
                )],
            )
            .await
            .unwrap();
        assert_eq!(n, 1);

        // Wrong category matches nothing.
        let miss = store
            .apply_enrichment(
                ReferenceCategory::Weapon,
                "ship_matrix",
                &[(
                    "AEGS_Avenger_Stalker".to_owned(),
                    serde_json::json!({ "ok": true }),
                )],
            )
            .await
            .unwrap();
        assert_eq!(miss, 0);
    }

    #[tokio::test]
    async fn apply_enrichment_refuses_empty_batch() {
        let store = MemoryReferenceStore::new();
        let err = store
            .apply_enrichment(ReferenceCategory::Vehicle, "ship_matrix", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, ReferenceStoreError::EmptyBatch));
    }

    #[tokio::test]
    async fn apply_enrichment_rejects_invalid_namespace() {
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[ReferenceEntry {
                category: ReferenceCategory::Vehicle,
                class_name: "AEGS_Avenger_Stalker".to_owned(),
                display_name: "Aegis Avenger Stalker".to_owned(),
                slug: None,
                metadata: serde_json::json!({}),
            }])
            .await
            .unwrap();

        for bad in ["", "ship.matrix", "ShipMatrix", "ship matrix", "ship2"] {
            let err = store
                .apply_enrichment(
                    ReferenceCategory::Vehicle,
                    bad,
                    &[("AEGS_Avenger_Stalker".to_owned(), serde_json::json!({}))],
                )
                .await
                .unwrap_err();
            assert!(
                matches!(err, ReferenceStoreError::InvalidNamespace(_)),
                "namespace {bad:?} should be rejected"
            );
        }
    }

    // ---- reconcile_category ----------------------------------------
    //
    // These exercise the in-memory semantics; the Postgres impl gets
    // its constraint-violation coverage at integration-test time
    // (where the real partial unique index on `(category, lower(slug))`
    // is present). The bug that motivated this method —
    // `reference_registry_cat_slug_lower_idx` tripping on cross-batch
    // collisions — is impossible by construction once stale rows are
    // gone, so the Memory impl asserts the behaviour without needing
    // to model the index itself.

    fn item(class_name: &str, slug: &str) -> ReferenceEntry {
        ReferenceEntry {
            category: ReferenceCategory::Item,
            class_name: class_name.to_owned(),
            display_name: class_name.to_owned(),
            slug: Some(slug.to_owned()),
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn reconcile_category_removes_stale_rows_and_inserts_fresh() {
        // The original bug: Day-1 sync inserted ITEM_Old with
        // slug=foo. Day-N sync sees a different class_name with the
        // same derived slug. Under the old additive upsert the new
        // row tripped `reference_registry_cat_slug_lower_idx`. With
        // reconcile, the stale row is removed BEFORE the upsert, so
        // the slug is free for reuse.
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[item("ITEM_Old", "foo")])
            .await
            .unwrap();

        let n = store
            .reconcile_category(
                ReferenceCategory::Item,
                "wiki_api",
                &[item("ITEM_New", "foo")],
            )
            .await
            .unwrap();
        assert_eq!(n, 1, "one row upserted");

        // Stale row is gone.
        assert!(store
            .get_entry(ReferenceCategory::Item, "ITEM_Old")
            .await
            .unwrap()
            .is_none());
        // Fresh row at the previously-conflicting slug.
        let got = store
            .get_by_slug(ReferenceCategory::Item, "foo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.class_name, "ITEM_New");
    }

    #[tokio::test]
    async fn reconcile_category_preserves_rows_that_survive_the_batch() {
        // Rows whose class_name appears in BOTH the prior DB and the
        // new batch must not be deleted-then-reinserted (would churn
        // updated_at); the upsert handles them in-place.
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[
                item("ITEM_A", "a"),
                item("ITEM_B", "b"),
                item("ITEM_C", "c"),
            ])
            .await
            .unwrap();

        store
            .reconcile_category(
                ReferenceCategory::Item,
                "wiki_api",
                &[
                    item("ITEM_A", "a"),
                    item("ITEM_B", "b-renamed"),
                    item("ITEM_D", "d"),
                ],
            )
            .await
            .unwrap();

        // ITEM_A unchanged, ITEM_B's slug updated, ITEM_C deleted,
        // ITEM_D inserted.
        assert!(store
            .get_entry(ReferenceCategory::Item, "ITEM_A")
            .await
            .unwrap()
            .is_some());
        let b = store
            .get_entry(ReferenceCategory::Item, "ITEM_B")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(b.slug.as_deref(), Some("b-renamed"));
        assert!(store
            .get_entry(ReferenceCategory::Item, "ITEM_C")
            .await
            .unwrap()
            .is_none());
        assert!(store
            .get_entry(ReferenceCategory::Item, "ITEM_D")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn reconcile_category_refuses_empty_batch() {
        // Empty batch is almost always a transient wiki outage.
        // Refuse the operation entirely so we don't nuke the
        // catalogue and surface as "everything disappeared" to users.
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[item("ITEM_A", "a"), item("ITEM_B", "b")])
            .await
            .unwrap();

        let err = store
            .reconcile_category(ReferenceCategory::Item, "wiki_api", &[])
            .await
            .expect_err("empty batch must be refused");
        assert!(matches!(err, ReferenceStoreError::EmptyBatch));

        // DB untouched.
        assert_eq!(
            store
                .list_category(ReferenceCategory::Item)
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn reconcile_category_scopes_deletion_to_its_own_category() {
        // A reconcile of `Item` MUST NOT delete `Vehicle` rows even
        // though Vehicle's class_names aren't in the Item batch.
        let store = MemoryReferenceStore::new();
        store
            .upsert_entries(&[
                item("ITEM_Old", "foo"),
                ReferenceEntry {
                    category: ReferenceCategory::Vehicle,
                    class_name: "VEH_Survivor".to_owned(),
                    display_name: "Survivor".to_owned(),
                    slug: Some("survivor".to_owned()),
                    metadata: serde_json::json!({}),
                },
            ])
            .await
            .unwrap();

        store
            .reconcile_category(
                ReferenceCategory::Item,
                "wiki_api",
                &[item("ITEM_New", "foo")],
            )
            .await
            .unwrap();

        // Vehicle row untouched.
        assert!(store
            .get_entry(ReferenceCategory::Vehicle, "VEH_Survivor")
            .await
            .unwrap()
            .is_some());
        // Item row swapped.
        assert!(store
            .get_entry(ReferenceCategory::Item, "ITEM_Old")
            .await
            .unwrap()
            .is_none());
        assert!(store
            .get_entry(ReferenceCategory::Item, "ITEM_New")
            .await
            .unwrap()
            .is_some());
    }
}
