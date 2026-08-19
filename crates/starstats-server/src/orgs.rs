//! Organizations metadata store + slug helpers.
//!
//! Ownership and membership for orgs is enforced by SpiceDB
//! (`organization:<slug>#owner|admin|member@user:<handle>`). This
//! module owns the small Postgres table that pairs each org's slug
//! with its display name + creator so the API can:
//!
//!  * list "orgs you own" without a SpiceDB ReadRelationships round
//!    trip (the common /v1/orgs landing case),
//!  * resolve `slug -> name` for any handler that needs the display
//!    name (e.g. share-with-org).
//!
//! The slug is the **identity** of the org in SpiceDB and never
//! changes after creation. Generation lives in [`slugify`] and the
//! collision-suffix loop lives in `org_routes::create_org`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Org {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub owner_user_id: Uuid,
    pub created_at: DateTime<Utc>,
    /// Surfaced by future "edit org" endpoints; today only `create`
    /// and `update_name` write it.
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum OrgError {
    #[error("organization slug already taken")]
    SlugTaken,
    /// Surfaced by handlers that promote `Ok(None)` from the store to
    /// a typed error. Today the route layer maps `Ok(None)` directly
    /// to 404 without going through this variant; kept on the enum
    /// for symmetry with `UserError`.
    #[allow(dead_code)]
    #[error("organization not found")]
    NotFound,
    /// Reserved for higher-level handlers that want to surface a
    /// typed forbidden response without leaking which check failed.
    /// The store layer itself doesn't enforce permissions — that is
    /// SpiceDB's job — but the variant lets callers use a shared
    /// error type.
    #[allow(dead_code)]
    #[error("forbidden")]
    Forbidden,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait OrgStore: Send + Sync + 'static {
    async fn create(&self, name: &str, slug: &str, owner_user_id: Uuid) -> Result<Org, OrgError>;

    async fn find_by_slug(&self, slug: &str) -> Result<Option<Org>, OrgError>;

    /// Surfaced by future "edit org name" + admin endpoints. The
    /// route layer today resolves orgs via slug only.
    #[allow(dead_code)]
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Org>, OrgError>;

    async fn list_for_owner(&self, user_id: Uuid) -> Result<Vec<Org>, OrgError>;

    /// Admin-only paginated list across ALL orgs. Substring match
    /// over name OR slug. Ordered by `created_at DESC` so new orgs
    /// surface first.
    async fn list_all(
        &self,
        q: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Org>, OrgError>;

    async fn delete_by_id(&self, id: Uuid) -> Result<(), OrgError>;

    /// Update the display name only. The slug stays put because it's
    /// the SpiceDB identity. Returns Ok(()) even if no row matched
    /// (idempotent — same posture as `delete`). Kept on the trait so
    /// the future "edit org" handler can call it without a re-write.
    #[allow(dead_code)]
    async fn update_name(&self, id: Uuid, name: &str) -> Result<(), OrgError>;
}

// -- Slug generation -------------------------------------------------

/// Maximum slug length. Chosen to fit comfortably in the SpiceDB
/// object_id string and to leave room for the `-N` collision suffix.
pub const SLUG_MAX_LEN: usize = 64;

/// Convert a free-form display name into a URL-safe lowercase slug.
///
/// Rules:
///  * Unicode letters/digits are folded to ASCII via the
///    "ASCII-or-skip" rule — the algorithm scans each char and keeps
///    only ASCII alphanumerics; everything else (including diacritics
///    and CJK) becomes a separator. This is intentionally simple; a
///    future iteration can swap in `unicode-normalization` if users
///    complain.
///  * Whitespace runs collapse to a single `-`.
///  * Leading/trailing `-` are trimmed.
///  * Truncated to [`SLUG_MAX_LEN`] characters. Truncation happens
///    *after* the collapse + trim so we don't leave a dangling `-`.
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else {
            // Treat any non-ASCII-alphanumeric as a separator. This
            // strips Unicode letters (e.g. é, 漢) — accepted as a
            // known limitation; see module docs.
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        }
    }
    // Trim trailing '-' that the collapser may have left behind.
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() > SLUG_MAX_LEN {
        out.truncate(SLUG_MAX_LEN);
        // Truncation can land mid-segment; tidy any dash that was
        // exposed at the new end.
        while out.ends_with('-') {
            out.pop();
        }
    }
    out
}

/// Append `-N` to `base`, never exceeding [`SLUG_MAX_LEN`]. Used by
/// the collision retry loop in `org_routes::create_org`.
pub fn slug_with_suffix(base: &str, n: u32) -> String {
    let suffix = format!("-{n}");
    let max_base = SLUG_MAX_LEN.saturating_sub(suffix.len());
    let mut s = String::with_capacity(SLUG_MAX_LEN);
    if base.len() > max_base {
        s.push_str(&base[..max_base]);
    } else {
        s.push_str(base);
    }
    while s.ends_with('-') {
        s.pop();
    }
    s.push_str(&suffix);
    s
}

// -- Postgres impl ---------------------------------------------------

pub struct PostgresOrgStore {
    pool: PgPool,
}

impl PostgresOrgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OrgStore for PostgresOrgStore {
    async fn create(&self, name: &str, slug: &str, owner_user_id: Uuid) -> Result<Org, OrgError> {
        let id = Uuid::new_v4();
        let row: Result<_, sqlx::Error> =
            sqlx::query_as::<_, (Uuid, String, String, Uuid, DateTime<Utc>, DateTime<Utc>)>(
                r#"
            INSERT INTO organizations (id, name, slug, owner_user_id)
            VALUES ($1, $2, $3, $4)
            RETURNING id, name, slug, owner_user_id, created_at, updated_at
            "#,
            )
            .bind(id)
            .bind(name)
            .bind(slug)
            .bind(owner_user_id)
            .fetch_one(&self.pool)
            .await;

        match row {
            Ok((id, name, slug, owner_user_id, created_at, updated_at)) => Ok(Org {
                id,
                name,
                slug,
                owner_user_id,
                created_at,
                updated_at,
            }),
            // The unique index on `slug` is the only way this INSERT
            // can hit a constraint; map it to the typed error the
            // route handler expects.
            Err(sqlx::Error::Database(db))
                if db.kind() == sqlx::error::ErrorKind::UniqueViolation =>
            {
                Err(OrgError::SlugTaken)
            }
            Err(e) => Err(OrgError::Database(e)),
        }
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<Org>, OrgError> {
        let row = sqlx::query_as::<_, (Uuid, String, String, Uuid, DateTime<Utc>, DateTime<Utc>)>(
            r#"
            SELECT id, name, slug, owner_user_id, created_at, updated_at
            FROM organizations
            WHERE slug = $1
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(id, name, slug, owner_user_id, created_at, updated_at)| Org {
                id,
                name,
                slug,
                owner_user_id,
                created_at,
                updated_at,
            },
        ))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Org>, OrgError> {
        let row = sqlx::query_as::<_, (Uuid, String, String, Uuid, DateTime<Utc>, DateTime<Utc>)>(
            r#"
            SELECT id, name, slug, owner_user_id, created_at, updated_at
            FROM organizations
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(id, name, slug, owner_user_id, created_at, updated_at)| Org {
                id,
                name,
                slug,
                owner_user_id,
                created_at,
                updated_at,
            },
        ))
    }

    async fn list_for_owner(&self, user_id: Uuid) -> Result<Vec<Org>, OrgError> {
        let rows = sqlx::query_as::<_, (Uuid, String, String, Uuid, DateTime<Utc>, DateTime<Utc>)>(
            r#"
            SELECT id, name, slug, owner_user_id, created_at, updated_at
            FROM organizations
            WHERE owner_user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(id, name, slug, owner_user_id, created_at, updated_at)| Org {
                    id,
                    name,
                    slug,
                    owner_user_id,
                    created_at,
                    updated_at,
                },
            )
            .collect())
    }

    async fn list_all(
        &self,
        q: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Org>, OrgError> {
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let q_norm = q.map(|s| s.trim()).filter(|s| !s.is_empty());

        let rows = if let Some(q) = q_norm {
            let pattern = format!("%{q}%");
            sqlx::query_as::<_, (Uuid, String, String, Uuid, DateTime<Utc>, DateTime<Utc>)>(
                r#"
                SELECT id, name, slug, owner_user_id, created_at, updated_at
                FROM organizations
                WHERE name ILIKE $1 OR slug ILIKE $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(pattern)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, (Uuid, String, String, Uuid, DateTime<Utc>, DateTime<Utc>)>(
                r#"
                SELECT id, name, slug, owner_user_id, created_at, updated_at
                FROM organizations
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows
            .into_iter()
            .map(
                |(id, name, slug, owner_user_id, created_at, updated_at)| Org {
                    id,
                    name,
                    slug,
                    owner_user_id,
                    created_at,
                    updated_at,
                },
            )
            .collect())
    }

    async fn delete_by_id(&self, id: Uuid) -> Result<(), OrgError> {
        sqlx::query("DELETE FROM organizations WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_name(&self, id: Uuid, name: &str) -> Result<(), OrgError> {
        sqlx::query(
            r#"
            UPDATE organizations
               SET name = $2,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(name)
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

    #[derive(Default)]
    pub struct MemoryOrgStore {
        // Keyed by slug for cheap unique-slug enforcement.
        rows: Mutex<HashMap<String, Org>>,
    }

    impl MemoryOrgStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl OrgStore for MemoryOrgStore {
        async fn create(
            &self,
            name: &str,
            slug: &str,
            owner_user_id: Uuid,
        ) -> Result<Org, OrgError> {
            let mut rows = self.rows.lock().unwrap();
            if rows.contains_key(slug) {
                return Err(OrgError::SlugTaken);
            }
            let now = Utc::now();
            let org = Org {
                id: Uuid::new_v4(),
                name: name.to_owned(),
                slug: slug.to_owned(),
                owner_user_id,
                created_at: now,
                updated_at: now,
            };
            rows.insert(slug.to_owned(), org.clone());
            Ok(org)
        }

        async fn find_by_slug(&self, slug: &str) -> Result<Option<Org>, OrgError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows.get(slug).cloned())
        }

        async fn find_by_id(&self, id: Uuid) -> Result<Option<Org>, OrgError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows.values().find(|o| o.id == id).cloned())
        }

        async fn list_for_owner(&self, user_id: Uuid) -> Result<Vec<Org>, OrgError> {
            let rows = self.rows.lock().unwrap();
            let mut out: Vec<Org> = rows
                .values()
                .filter(|o| o.owner_user_id == user_id)
                .cloned()
                .collect();
            out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(out)
        }

        async fn list_all(
            &self,
            q: Option<&str>,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<Org>, OrgError> {
            let limit = limit.clamp(1, 200) as usize;
            let offset = offset.max(0) as usize;
            let q_lower = q.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
            let rows = self.rows.lock().unwrap();
            let mut out: Vec<Org> = rows
                .values()
                .filter(|o| match q_lower.as_ref() {
                    None => true,
                    Some(q) => {
                        o.name.to_lowercase().contains(q) || o.slug.to_lowercase().contains(q)
                    }
                })
                .cloned()
                .collect();
            out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(out.into_iter().skip(offset).take(limit).collect())
        }

        async fn delete_by_id(&self, id: Uuid) -> Result<(), OrgError> {
            let mut rows = self.rows.lock().unwrap();
            rows.retain(|_, o| o.id != id);
            Ok(())
        }

        async fn update_name(&self, id: Uuid, name: &str) -> Result<(), OrgError> {
            let mut rows = self.rows.lock().unwrap();
            for o in rows.values_mut() {
                if o.id == id {
                    o.name = name.to_owned();
                    o.updated_at = Utc::now();
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryOrgStore;
    use super::*;

    #[test]
    fn slugify_basic_collapses_whitespace() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("  Hello   World  "), "hello-world");
    }

    #[test]
    fn slugify_strips_unicode() {
        // Unicode letters get treated as separators (the ASCII-only
        // rule). This is documented; the homestead acceptance is
        // "good enough" rather than "lossless".
        assert_eq!(slugify("Café Latté"), "caf-latt");
        assert_eq!(slugify("漢字 stars"), "stars");
    }

    #[test]
    fn slugify_collapses_whitespace_and_punctuation() {
        assert_eq!(slugify("Foo!! Bar??  baz"), "foo-bar-baz");
        assert_eq!(slugify("A---B"), "a-b");
        assert_eq!(slugify("---trim---me---"), "trim-me");
    }

    #[test]
    fn slugify_truncates_at_64_chars() {
        let long = "a".repeat(100);
        let s = slugify(&long);
        assert!(s.len() <= SLUG_MAX_LEN);
        assert_eq!(s.len(), SLUG_MAX_LEN);
    }

    #[test]
    fn slugify_returns_empty_for_empty_input() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("    "), "");
        assert_eq!(slugify("漢字"), "");
    }

    #[test]
    fn slug_with_suffix_keeps_under_limit() {
        let base = "a".repeat(SLUG_MAX_LEN);
        let s = slug_with_suffix(&base, 9);
        assert!(s.len() <= SLUG_MAX_LEN);
        assert!(s.ends_with("-9"));
    }

    #[tokio::test]
    async fn create_then_find_by_slug_round_trips() {
        let store = MemoryOrgStore::new();
        let owner = Uuid::new_v4();
        let org = store.create("Cool Org", "cool-org", owner).await.unwrap();
        let found = store.find_by_slug("cool-org").await.unwrap().unwrap();
        assert_eq!(found.id, org.id);
        assert_eq!(found.name, "Cool Org");
        assert_eq!(found.slug, "cool-org");
        assert_eq!(found.owner_user_id, owner);
    }

    #[tokio::test]
    async fn slug_uniqueness_enforced() {
        let store = MemoryOrgStore::new();
        let owner = Uuid::new_v4();
        store.create("First", "shared", owner).await.unwrap();
        let err = store.create("Second", "shared", owner).await.unwrap_err();
        assert!(matches!(err, OrgError::SlugTaken));
    }

    #[tokio::test]
    async fn delete_removes_org() {
        let store = MemoryOrgStore::new();
        let owner = Uuid::new_v4();
        let org = store.create("Bye", "bye-org", owner).await.unwrap();
        store.delete_by_id(org.id).await.unwrap();
        assert!(store.find_by_slug("bye-org").await.unwrap().is_none());
        assert!(store.find_by_id(org.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_for_owner_returns_only_owned() {
        let store = MemoryOrgStore::new();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        store.create("A1", "a1", alice).await.unwrap();
        store.create("A2", "a2", alice).await.unwrap();
        store.create("B1", "b1", bob).await.unwrap();
        let alice_orgs = store.list_for_owner(alice).await.unwrap();
        assert_eq!(alice_orgs.len(), 2);
        assert!(alice_orgs.iter().all(|o| o.owner_user_id == alice));
    }

    #[tokio::test]
    async fn update_name_changes_display_only() {
        let store = MemoryOrgStore::new();
        let owner = Uuid::new_v4();
        let org = store.create("Old", "stable-slug", owner).await.unwrap();
        store.update_name(org.id, "New").await.unwrap();
        let found = store.find_by_slug("stable-slug").await.unwrap().unwrap();
        assert_eq!(found.name, "New");
        assert_eq!(found.slug, "stable-slug");
    }
}
