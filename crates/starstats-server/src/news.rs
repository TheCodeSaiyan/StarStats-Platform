//! News posts written by staff and read by every player (migration 0071).
//!
//! A post is stored once. Per-player state is only which posts a player has
//! opened (`news_reads`), so publishing to everyone costs one row, not one
//! per user. Bodies are plain text and are never rendered as HTML.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;
use utoipa::ToSchema;
use uuid::Uuid;

pub const TITLE_MAX: usize = 120;
pub const BODY_MAX: usize = 4000;
pub const LINK_MAX: usize = 500;
/// Upper bound on any list read.
pub const LIST_MAX: i64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewsPost {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub link_url: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// `None` = draft.
    pub published_at: Option<DateTime<Utc>>,
}

/// A validated draft. Constructed only through [`NewsDraft::parse`], so a
/// store never sees an over-long title or a non-https link.
#[derive(Debug, Clone, PartialEq)]
pub struct NewsDraft {
    pub title: String,
    pub body: String,
    pub link_url: Option<String>,
}

impl NewsDraft {
    /// Trim and check. The error is the API error code.
    pub fn parse(title: &str, body: &str, link_url: Option<&str>) -> Result<Self, &'static str> {
        let title = title.trim();
        let body = body.trim();
        if title.is_empty() {
            return Err("title_required");
        }
        if title.chars().count() > TITLE_MAX {
            return Err("title_too_long");
        }
        if body.is_empty() {
            return Err("body_required");
        }
        if body.chars().count() > BODY_MAX {
            return Err("body_too_long");
        }
        let link_url = match link_url.map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(l) => {
                // https only: the tray opens it in the system browser and
                // the web renders it as a link, and neither should ever
                // follow `javascript:` or plain http.
                if !l.starts_with("https://") || l.chars().count() > LINK_MAX || l.contains(' ') {
                    return Err("invalid_link");
                }
                Some(l.to_string())
            }
        };
        Ok(Self {
            title: title.to_string(),
            body: body.to_string(),
            link_url,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NewsError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("news post not found")]
    NotFound,
}

#[async_trait]
pub trait NewsStore: Send + Sync + 'static {
    async fn create(
        &self,
        draft: &NewsDraft,
        author: &str,
        publish: bool,
    ) -> Result<NewsPost, NewsError>;
    async fn update(&self, id: Uuid, draft: &NewsDraft) -> Result<NewsPost, NewsError>;
    /// Publish (stamping now, unless already published) or return to draft.
    async fn set_published(&self, id: Uuid, published: bool) -> Result<NewsPost, NewsError>;
    async fn delete(&self, id: Uuid) -> Result<(), NewsError>;
    /// Every live post including drafts, newest first. Admin only.
    async fn list_all(&self, limit: i64) -> Result<Vec<NewsPost>, NewsError>;
    /// Published posts, newest published first.
    async fn list_published(&self, limit: i64) -> Result<Vec<NewsPost>, NewsError>;
    async fn mark_seen(&self, user_id: Uuid, news_id: Uuid) -> Result<(), NewsError>;
    /// Which of `ids` this user has opened.
    async fn seen_ids(&self, user_id: Uuid, ids: &[Uuid]) -> Result<HashSet<Uuid>, NewsError>;
}

pub struct PostgresNewsStore {
    pool: PgPool,
}

impl PostgresNewsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type Row = (
    Uuid,
    String,
    String,
    Option<String>,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

fn to_post(r: Row) -> NewsPost {
    NewsPost {
        id: r.0,
        title: r.1,
        body: r.2,
        link_url: r.3,
        created_by: r.4,
        created_at: r.5,
        updated_at: r.6,
        published_at: r.7,
    }
}

const COLS: &str = "id, title, body, link_url, created_by, created_at, updated_at, published_at";

#[async_trait]
impl NewsStore for PostgresNewsStore {
    async fn create(
        &self,
        draft: &NewsDraft,
        author: &str,
        publish: bool,
    ) -> Result<NewsPost, NewsError> {
        let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO news_posts (title, body, link_url, created_by, published_at)
             VALUES ($1, $2, $3, $4, CASE WHEN $5 THEN NOW() ELSE NULL END)
             RETURNING {COLS}"
        )))
        .bind(&draft.title)
        .bind(&draft.body)
        .bind(&draft.link_url)
        .bind(author)
        .bind(publish)
        .fetch_one(&self.pool)
        .await?;
        Ok(to_post(row))
    }

    async fn update(&self, id: Uuid, draft: &NewsDraft) -> Result<NewsPost, NewsError> {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE news_posts SET title = $2, body = $3, link_url = $4, updated_at = NOW()
             WHERE id = $1 AND deleted_at IS NULL
             RETURNING {COLS}"
        )))
        .bind(id)
        .bind(&draft.title)
        .bind(&draft.body)
        .bind(&draft.link_url)
        .fetch_optional(&self.pool)
        .await?;
        row.map(to_post).ok_or(NewsError::NotFound)
    }

    async fn set_published(&self, id: Uuid, published: bool) -> Result<NewsPost, NewsError> {
        // COALESCE keeps the ORIGINAL publish time on a re-publish of a
        // post that is already live, so a no-op click does not bump it to
        // the top of everyone's feed.
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE news_posts
             SET published_at = CASE WHEN $2 THEN COALESCE(published_at, NOW()) ELSE NULL END,
                 updated_at = NOW()
             WHERE id = $1 AND deleted_at IS NULL
             RETURNING {COLS}"
        )))
        .bind(id)
        .bind(published)
        .fetch_optional(&self.pool)
        .await?;
        row.map(to_post).ok_or(NewsError::NotFound)
    }

    async fn delete(&self, id: Uuid) -> Result<(), NewsError> {
        let res = sqlx::query(
            "UPDATE news_posts SET deleted_at = NOW() WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(NewsError::NotFound);
        }
        Ok(())
    }

    async fn list_all(&self, limit: i64) -> Result<Vec<NewsPost>, NewsError> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM news_posts WHERE deleted_at IS NULL
             ORDER BY created_at DESC LIMIT $1"
        )))
        .bind(limit.clamp(1, LIST_MAX))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(to_post).collect())
    }

    async fn list_published(&self, limit: i64) -> Result<Vec<NewsPost>, NewsError> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM news_posts
             WHERE deleted_at IS NULL AND published_at IS NOT NULL
             ORDER BY published_at DESC LIMIT $1"
        )))
        .bind(limit.clamp(1, LIST_MAX))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(to_post).collect())
    }

    async fn mark_seen(&self, user_id: Uuid, news_id: Uuid) -> Result<(), NewsError> {
        sqlx::query(
            "INSERT INTO news_reads (user_id, news_id) VALUES ($1, $2)
             ON CONFLICT (user_id, news_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(news_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn seen_ids(&self, user_id: Uuid, ids: &[Uuid]) -> Result<HashSet<Uuid>, NewsError> {
        if ids.is_empty() {
            return Ok(HashSet::new());
        }
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT news_id FROM news_reads WHERE user_id = $1 AND news_id = ANY($2)",
        )
        .bind(user_id)
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryNewsStore {
        posts: Mutex<Vec<(NewsPost, bool)>>,
        reads: Mutex<HashSet<(Uuid, Uuid)>>,
    }

    impl MemoryNewsStore {
        pub fn new() -> Self {
            Self::default()
        }

        fn live<T>(&self, id: Uuid, f: impl FnOnce(&mut NewsPost) -> T) -> Result<T, NewsError> {
            let mut g = self.posts.lock().unwrap();
            let (p, _) = g
                .iter_mut()
                .find(|(p, deleted)| p.id == id && !*deleted)
                .ok_or(NewsError::NotFound)?;
            Ok(f(p))
        }
    }

    #[async_trait]
    impl NewsStore for MemoryNewsStore {
        async fn create(
            &self,
            draft: &NewsDraft,
            author: &str,
            publish: bool,
        ) -> Result<NewsPost, NewsError> {
            let now = Utc::now();
            let p = NewsPost {
                id: Uuid::now_v7(),
                title: draft.title.clone(),
                body: draft.body.clone(),
                link_url: draft.link_url.clone(),
                created_by: author.into(),
                created_at: now,
                updated_at: now,
                published_at: publish.then_some(now),
            };
            self.posts.lock().unwrap().push((p.clone(), false));
            Ok(p)
        }

        async fn update(&self, id: Uuid, draft: &NewsDraft) -> Result<NewsPost, NewsError> {
            self.live(id, |p| {
                p.title = draft.title.clone();
                p.body = draft.body.clone();
                p.link_url = draft.link_url.clone();
                p.updated_at = Utc::now();
                p.clone()
            })
        }

        async fn set_published(&self, id: Uuid, published: bool) -> Result<NewsPost, NewsError> {
            self.live(id, |p| {
                p.published_at = if published {
                    Some(p.published_at.unwrap_or_else(Utc::now))
                } else {
                    None
                };
                p.clone()
            })
        }

        async fn delete(&self, id: Uuid) -> Result<(), NewsError> {
            let mut g = self.posts.lock().unwrap();
            let row = g
                .iter_mut()
                .find(|(p, deleted)| p.id == id && !*deleted)
                .ok_or(NewsError::NotFound)?;
            row.1 = true;
            Ok(())
        }

        async fn list_all(&self, limit: i64) -> Result<Vec<NewsPost>, NewsError> {
            let mut v: Vec<NewsPost> = self
                .posts
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, d)| !d)
                .map(|(p, _)| p.clone())
                .collect();
            v.sort_by_key(|p| std::cmp::Reverse(p.created_at));
            v.truncate(limit.clamp(1, LIST_MAX) as usize);
            Ok(v)
        }

        async fn list_published(&self, limit: i64) -> Result<Vec<NewsPost>, NewsError> {
            let mut v: Vec<NewsPost> = self
                .list_all(LIST_MAX)
                .await?
                .into_iter()
                .filter(|p| p.published_at.is_some())
                .collect();
            v.sort_by_key(|p| std::cmp::Reverse(p.published_at));
            v.truncate(limit.clamp(1, LIST_MAX) as usize);
            Ok(v)
        }

        async fn mark_seen(&self, user_id: Uuid, news_id: Uuid) -> Result<(), NewsError> {
            self.reads.lock().unwrap().insert((user_id, news_id));
            Ok(())
        }

        async fn seen_ids(&self, user_id: Uuid, ids: &[Uuid]) -> Result<HashSet<Uuid>, NewsError> {
            let g = self.reads.lock().unwrap();
            Ok(ids
                .iter()
                .copied()
                .filter(|id| g.contains(&(user_id, *id)))
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryNewsStore;
    use super::*;

    fn draft(t: &str) -> NewsDraft {
        NewsDraft::parse(t, "Body text.", None).unwrap()
    }

    #[test]
    fn draft_validation() {
        assert_eq!(NewsDraft::parse(" ", "b", None), Err("title_required"));
        assert_eq!(NewsDraft::parse("t", "  ", None), Err("body_required"));
        assert_eq!(
            NewsDraft::parse(&"x".repeat(TITLE_MAX + 1), "b", None),
            Err("title_too_long")
        );
        assert_eq!(
            NewsDraft::parse("t", &"x".repeat(BODY_MAX + 1), None),
            Err("body_too_long")
        );
        for bad in ["http://starstats.app", "javascript:alert(1)", "https://a b"] {
            assert_eq!(
                NewsDraft::parse("t", "b", Some(bad)),
                Err("invalid_link"),
                "{bad}"
            );
        }
        let ok = NewsDraft::parse("  Title ", " Body ", Some(" https://starstats.app/x ")).unwrap();
        assert_eq!(ok.title, "Title");
        assert_eq!(ok.link_url.as_deref(), Some("https://starstats.app/x"));
        assert_eq!(
            NewsDraft::parse("t", "b", Some("  ")).unwrap().link_url,
            None
        );
    }

    #[tokio::test]
    async fn drafts_are_not_published_until_published() {
        let s = MemoryNewsStore::new();
        let d = s.create(&draft("Draft"), "mod", false).await.unwrap();
        s.create(&draft("Live"), "mod", true).await.unwrap();
        assert_eq!(s.list_all(10).await.unwrap().len(), 2);
        let live = s.list_published(10).await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].title, "Live");
        s.set_published(d.id, true).await.unwrap();
        assert_eq!(s.list_published(10).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn republishing_keeps_the_original_time_and_unpublish_clears_it() {
        let s = MemoryNewsStore::new();
        let p = s.create(&draft("Live"), "mod", true).await.unwrap();
        let again = s.set_published(p.id, true).await.unwrap();
        assert_eq!(again.published_at, p.published_at);
        let off = s.set_published(p.id, false).await.unwrap();
        assert!(off.published_at.is_none());
    }

    #[tokio::test]
    async fn deleted_posts_vanish_and_cannot_be_edited() {
        let s = MemoryNewsStore::new();
        let p = s.create(&draft("Gone"), "mod", true).await.unwrap();
        s.delete(p.id).await.unwrap();
        assert!(s.list_all(10).await.unwrap().is_empty());
        assert!(matches!(
            s.update(p.id, &draft("x")).await,
            Err(NewsError::NotFound)
        ));
        assert!(matches!(s.delete(p.id).await, Err(NewsError::NotFound)));
    }

    #[tokio::test]
    async fn read_state_is_per_user() {
        let s = MemoryNewsStore::new();
        let p = s.create(&draft("Live"), "mod", true).await.unwrap();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        s.mark_seen(a, p.id).await.unwrap();
        s.mark_seen(a, p.id).await.unwrap();
        assert!(s.seen_ids(a, &[p.id]).await.unwrap().contains(&p.id));
        assert!(s.seen_ids(b, &[p.id]).await.unwrap().is_empty());
    }
}
/// Postgres round trip for every `PostgresNewsStore` method, gated on
/// STARSTATS_TEST_DATABASE_URL like the other round-trip tests. The Memory
/// store cannot catch a bound bool Postgres will not type, or a partial
/// index the ORDER BY misses.
#[cfg(test)]
mod postgres_tests {
    use super::*;

    #[tokio::test]
    async fn postgres_news_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres round-trip test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");
        sqlx::query("DELETE FROM news_posts WHERE created_by = 'news_probe'")
            .execute(&pool)
            .await
            .unwrap();

        let s = PostgresNewsStore::new(pool.clone());
        let d = NewsDraft::parse("Probe draft", "Body.", Some("https://starstats.app")).unwrap();
        let draft = s.create(&d, "news_probe", false).await.unwrap();
        assert!(draft.published_at.is_none());
        let live = s.create(&d, "news_probe", true).await.unwrap();
        assert!(live.published_at.is_some());

        let published = s.list_published(100).await.unwrap();
        assert!(published.iter().any(|p| p.id == live.id));
        assert!(!published.iter().any(|p| p.id == draft.id));

        let again = s.set_published(live.id, true).await.unwrap();
        assert_eq!(
            again.published_at, live.published_at,
            "re-publish keeps the time"
        );
        let off = s.set_published(live.id, false).await.unwrap();
        assert!(off.published_at.is_none());

        let edited = s
            .update(
                draft.id,
                &NewsDraft::parse("Edited", "New body.", None).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(edited.title, "Edited");
        assert!(edited.link_url.is_none());

        let user = Uuid::now_v7();
        s.mark_seen(user, draft.id).await.unwrap();
        s.mark_seen(user, draft.id).await.unwrap();
        let seen = s.seen_ids(user, &[draft.id, live.id]).await.unwrap();
        assert!(seen.contains(&draft.id) && !seen.contains(&live.id));

        s.delete(draft.id).await.unwrap();
        assert!(matches!(s.delete(draft.id).await, Err(NewsError::NotFound)));
        assert!(!s
            .list_all(100)
            .await
            .unwrap()
            .iter()
            .any(|p| p.id == draft.id));

        sqlx::query("DELETE FROM news_reads WHERE user_id = $1")
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM news_posts WHERE created_by = 'news_probe'")
            .execute(&pool)
            .await
            .unwrap();
    }
}
