//! Release notes, one row per release tag (migration 0072).
//!
//! Written by CI after a release is tagged; read by the tray's What's New,
//! the web's /whats-new and /changelog. The notes are generated from commit
//! subjects by scripts/release-notes.mjs, so there is no editing here: a
//! re-sent release replaces its row.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;
use utoipa::ToSchema;
use uuid::Uuid;

pub const LIST_MAX: i64 = 50;

pub const TRACKS: &[&str] = &["tray", "platform"];
pub const CHANNELS: &[&str] = &["alpha", "beta", "rc", "live"];

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Release {
    pub id: Uuid,
    /// `tray` or `platform`.
    pub track: String,
    pub tag: String,
    pub version: String,
    /// `alpha`, `beta`, `rc` or `live`.
    pub channel: String,
    pub released_on: NaiveDate,
    /// "5 new, 1 improved, 1 fixed". Empty when nothing player-facing.
    pub summary: String,
    /// Groups exactly as the generator emits them:
    /// `[{kind, lines: [{text, surfaces, prs, roadmap}]}]`.
    #[schema(value_type = Object)]
    pub notes: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A validated release write. Built only through [`ReleaseWrite::parse`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseWrite {
    pub track: String,
    pub tag: String,
    pub version: String,
    pub channel: String,
    pub released_on: NaiveDate,
    pub summary: String,
    pub notes: serde_json::Value,
}

impl ReleaseWrite {
    /// The error is a short reason for the CI log.
    pub fn parse(
        track: &str,
        tag: &str,
        version: &str,
        channel: &str,
        released_on: &str,
        summary: &str,
        notes: serde_json::Value,
    ) -> Result<Self, &'static str> {
        if !TRACKS.contains(&track) {
            return Err("unknown track");
        }
        if !CHANNELS.contains(&channel) {
            return Err("unknown channel");
        }
        let expected_prefix = if track == "tray" { "tray-v" } else { "v" };
        if !tag.starts_with(expected_prefix) || tag.len() > 64 || !tag.contains(version) {
            return Err("tag does not match track and version");
        }
        if version.is_empty()
            || version.len() > 32
            || !version.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            return Err("invalid version");
        }
        let released_on =
            NaiveDate::parse_from_str(released_on, "%Y-%m-%d").map_err(|_| "invalid date")?;
        if summary.chars().count() > 200 {
            return Err("summary too long");
        }
        if !notes.is_array() {
            return Err("notes must be an array");
        }
        // Generous for a release, small enough that a runaway payload
        // cannot fill a row.
        if notes.to_string().len() > 64 * 1024 {
            return Err("notes too large");
        }
        Ok(Self {
            track: track.into(),
            tag: tag.into(),
            version: version.into(),
            channel: channel.into(),
            released_on,
            summary: summary.into(),
            notes,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait ReleaseStore: Send + Sync + 'static {
    /// Insert, or replace the row with the same tag.
    async fn upsert(&self, r: &ReleaseWrite) -> Result<Release, ReleaseError>;
    /// Newest first. `track` None = both; `channels` empty = all.
    async fn list(
        &self,
        track: Option<&str>,
        channels: &[String],
        limit: i64,
    ) -> Result<Vec<Release>, ReleaseError>;
    async fn mark_seen(&self, user_id: Uuid, release_id: Uuid) -> Result<(), ReleaseError>;
    async fn seen_ids(&self, user_id: Uuid, ids: &[Uuid]) -> Result<HashSet<Uuid>, ReleaseError>;
}

pub struct PostgresReleaseStore {
    pool: PgPool,
}

impl PostgresReleaseStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type Row = (
    Uuid,
    String,
    String,
    String,
    String,
    NaiveDate,
    String,
    serde_json::Value,
    DateTime<Utc>,
);

fn to_release(r: Row) -> Release {
    Release {
        id: r.0,
        track: r.1,
        tag: r.2,
        version: r.3,
        channel: r.4,
        released_on: r.5,
        summary: r.6,
        notes: r.7,
        created_at: r.8,
    }
}

const COLS: &str = "id, track, tag, version, channel, released_on, summary, notes, created_at";

#[async_trait]
impl ReleaseStore for PostgresReleaseStore {
    async fn upsert(&self, r: &ReleaseWrite) -> Result<Release, ReleaseError> {
        let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO releases (track, tag, version, channel, released_on, summary, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tag) DO UPDATE SET
                 track = EXCLUDED.track,
                 version = EXCLUDED.version,
                 channel = EXCLUDED.channel,
                 released_on = EXCLUDED.released_on,
                 summary = EXCLUDED.summary,
                 notes = EXCLUDED.notes,
                 updated_at = NOW()
             RETURNING {COLS}"
        )))
        .bind(&r.track)
        .bind(&r.tag)
        .bind(&r.version)
        .bind(&r.channel)
        .bind(r.released_on)
        .bind(&r.summary)
        .bind(&r.notes)
        .fetch_one(&self.pool)
        .await?;
        Ok(to_release(row))
    }

    async fn list(
        &self,
        track: Option<&str>,
        channels: &[String],
        limit: i64,
    ) -> Result<Vec<Release>, ReleaseError> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM releases
             WHERE ($1::text IS NULL OR track = $1)
               AND (cardinality($2::text[]) = 0 OR channel = ANY($2))
             ORDER BY released_on DESC, created_at DESC
             LIMIT $3"
        )))
        .bind(track)
        .bind(channels)
        .bind(limit.clamp(1, LIST_MAX))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(to_release).collect())
    }

    async fn mark_seen(&self, user_id: Uuid, release_id: Uuid) -> Result<(), ReleaseError> {
        sqlx::query(
            "INSERT INTO release_reads (user_id, release_id) VALUES ($1, $2)
             ON CONFLICT (user_id, release_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(release_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn seen_ids(&self, user_id: Uuid, ids: &[Uuid]) -> Result<HashSet<Uuid>, ReleaseError> {
        if ids.is_empty() {
            return Ok(HashSet::new());
        }
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT release_id FROM release_reads WHERE user_id = $1 AND release_id = ANY($2)",
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
    pub struct MemoryReleaseStore {
        rows: Mutex<Vec<Release>>,
        reads: Mutex<HashSet<(Uuid, Uuid)>>,
    }

    impl MemoryReleaseStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl ReleaseStore for MemoryReleaseStore {
        async fn upsert(&self, r: &ReleaseWrite) -> Result<Release, ReleaseError> {
            let mut g = self.rows.lock().unwrap();
            let existing = g.iter().position(|x| x.tag == r.tag);
            let rel = Release {
                id: existing.map(|i| g[i].id).unwrap_or_else(Uuid::now_v7),
                track: r.track.clone(),
                tag: r.tag.clone(),
                version: r.version.clone(),
                channel: r.channel.clone(),
                released_on: r.released_on,
                summary: r.summary.clone(),
                notes: r.notes.clone(),
                created_at: existing.map(|i| g[i].created_at).unwrap_or_else(Utc::now),
            };
            match existing {
                Some(i) => g[i] = rel.clone(),
                None => g.push(rel.clone()),
            }
            Ok(rel)
        }

        async fn list(
            &self,
            track: Option<&str>,
            channels: &[String],
            limit: i64,
        ) -> Result<Vec<Release>, ReleaseError> {
            let mut v: Vec<Release> = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|r| track.map(|t| r.track == t).unwrap_or(true))
                .filter(|r| channels.is_empty() || channels.contains(&r.channel))
                .cloned()
                .collect();
            v.sort_by(|a, b| {
                b.released_on
                    .cmp(&a.released_on)
                    .then(b.created_at.cmp(&a.created_at))
            });
            v.truncate(limit.clamp(1, LIST_MAX) as usize);
            Ok(v)
        }

        async fn mark_seen(&self, user_id: Uuid, release_id: Uuid) -> Result<(), ReleaseError> {
            self.reads.lock().unwrap().insert((user_id, release_id));
            Ok(())
        }

        async fn seen_ids(
            &self,
            user_id: Uuid,
            ids: &[Uuid],
        ) -> Result<HashSet<Uuid>, ReleaseError> {
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
    use super::test_support::MemoryReleaseStore;
    use super::*;
    use serde_json::json;

    fn w(track: &str, tag: &str, version: &str, channel: &str, day: &str) -> ReleaseWrite {
        ReleaseWrite::parse(track, tag, version, channel, day, "1 new", json!([])).unwrap()
    }

    #[test]
    fn writes_are_validated() {
        let ok = |t, tag, v, c, d| ReleaseWrite::parse(t, tag, v, c, d, "", json!([]));
        assert!(ok("tray", "tray-v0.1.31", "0.1.31", "live", "2026-09-26").is_ok());
        assert!(ok(
            "platform",
            "v0.1.62-alpha.1",
            "0.1.62",
            "alpha",
            "2026-09-26"
        )
        .is_ok());
        assert_eq!(
            ok("desktop", "tray-v0.1.31", "0.1.31", "live", "2026-09-26"),
            Err("unknown track")
        );
        assert_eq!(
            ok("tray", "tray-v0.1.31", "0.1.31", "nightly", "2026-09-26"),
            Err("unknown channel")
        );
        assert_eq!(
            ok("tray", "v0.1.31", "0.1.31", "live", "2026-09-26"),
            Err("tag does not match track and version")
        );
        assert_eq!(
            ok("tray", "tray-v0.1.31", "0.1.30", "live", "2026-09-26"),
            Err("tag does not match track and version")
        );
        assert_eq!(
            ok("tray", "tray-v0.1.31", "0.1.31", "live", "26/09/2026"),
            Err("invalid date")
        );
        assert_eq!(
            ReleaseWrite::parse(
                "tray",
                "tray-v0.1.31",
                "0.1.31",
                "live",
                "2026-09-26",
                "",
                json!({})
            ),
            Err("notes must be an array")
        );
    }

    #[tokio::test]
    async fn upsert_replaces_by_tag() {
        let s = MemoryReleaseStore::new();
        let a = s
            .upsert(&w("tray", "tray-v0.1.31", "0.1.31", "live", "2026-09-26"))
            .await
            .unwrap();
        let mut again = w("tray", "tray-v0.1.31", "0.1.31", "live", "2026-09-26");
        again.summary = "2 new".into();
        let b = s.upsert(&again).await.unwrap();
        assert_eq!(a.id, b.id, "same row");
        let all = s.list(None, &[], 10).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].summary, "2 new");
    }

    #[tokio::test]
    async fn list_filters_track_and_channels_newest_first() {
        let s = MemoryReleaseStore::new();
        s.upsert(&w("tray", "tray-v0.1.30", "0.1.30", "live", "2026-09-21"))
            .await
            .unwrap();
        s.upsert(&w(
            "tray",
            "tray-v0.1.31-alpha.1",
            "0.1.31",
            "alpha",
            "2026-09-26",
        ))
        .await
        .unwrap();
        s.upsert(&w("tray", "tray-v0.1.31", "0.1.31", "live", "2026-09-26"))
            .await
            .unwrap();
        s.upsert(&w("platform", "v0.1.62", "0.1.62", "live", "2026-09-26"))
            .await
            .unwrap();

        let tray_live = s.list(Some("tray"), &["live".into()], 10).await.unwrap();
        assert_eq!(
            tray_live.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(),
            vec!["tray-v0.1.31", "tray-v0.1.30"]
        );
        assert_eq!(s.list(Some("tray"), &[], 10).await.unwrap().len(), 3);
        assert_eq!(s.list(None, &[], 10).await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn read_state_is_per_user() {
        let s = MemoryReleaseStore::new();
        let r = s
            .upsert(&w("tray", "tray-v0.1.31", "0.1.31", "live", "2026-09-26"))
            .await
            .unwrap();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        s.mark_seen(a, r.id).await.unwrap();
        s.mark_seen(a, r.id).await.unwrap();
        assert!(s.seen_ids(a, &[r.id]).await.unwrap().contains(&r.id));
        assert!(s.seen_ids(b, &[r.id]).await.unwrap().is_empty());
    }
}
/// Postgres round trip for `PostgresReleaseStore`, gated on
/// STARSTATS_TEST_DATABASE_URL like the other round-trip tests. Catches what
/// the Memory store cannot: the `cardinality($2::text[])` empty-filter case,
/// the ON CONFLICT (tag) target, and DATE binding.
#[cfg(test)]
mod postgres_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn postgres_release_round_trip() {
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
        sqlx::query("DELETE FROM releases WHERE tag LIKE 'tray-v9.%' OR tag LIKE 'v9.%'")
            .execute(&pool)
            .await
            .unwrap();

        let s = PostgresReleaseStore::new(pool.clone());
        let w = |tag: &str, version: &str, channel: &str, day: &str| {
            let track = if tag.starts_with("tray-") {
                "tray"
            } else {
                "platform"
            };
            ReleaseWrite::parse(
                track,
                tag,
                version,
                channel,
                day,
                "1 new",
                json!([{"kind": "New", "lines": []}]),
            )
            .unwrap()
        };
        let a = s
            .upsert(&w("tray-v9.0.1", "9.0.1", "live", "2026-09-20"))
            .await
            .unwrap();
        s.upsert(&w("tray-v9.0.2-alpha.1", "9.0.2", "alpha", "2026-09-26"))
            .await
            .unwrap();
        s.upsert(&w("v9.0.2", "9.0.2", "live", "2026-09-26"))
            .await
            .unwrap();
        let mut again = w("tray-v9.0.1", "9.0.1", "live", "2026-09-20");
        again.summary = "2 new".into();
        let b = s.upsert(&again).await.unwrap();
        assert_eq!(a.id, b.id, "ON CONFLICT (tag) keeps the row");
        assert_eq!(b.summary, "2 new");
        assert_eq!(b.notes[0]["kind"], "New");

        let tray_all = s.list(Some("tray"), &[], 50).await.unwrap();
        let mine: Vec<&str> = tray_all
            .iter()
            .map(|r| r.tag.as_str())
            .filter(|t| t.starts_with("tray-v9."))
            .collect();
        assert_eq!(
            mine,
            vec!["tray-v9.0.2-alpha.1", "tray-v9.0.1"],
            "empty channel filter = all, newest first"
        );
        let tray_live = s.list(Some("tray"), &["live".into()], 50).await.unwrap();
        assert!(tray_live.iter().all(|r| r.channel == "live"));
        assert!(tray_live.iter().any(|r| r.tag == "tray-v9.0.1"));
        assert!(!tray_live.iter().any(|r| r.tag == "tray-v9.0.2-alpha.1"));
        assert!(s
            .list(None, &[], 50)
            .await
            .unwrap()
            .iter()
            .any(|r| r.tag == "v9.0.2"));

        let user = Uuid::now_v7();
        s.mark_seen(user, a.id).await.unwrap();
        s.mark_seen(user, a.id).await.unwrap();
        assert!(s.seen_ids(user, &[a.id]).await.unwrap().contains(&a.id));

        sqlx::query("DELETE FROM release_reads WHERE user_id = $1")
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM releases WHERE tag LIKE 'tray-v9.%' OR tag LIKE 'v9.%'")
            .execute(&pool)
            .await
            .unwrap();
    }
}
