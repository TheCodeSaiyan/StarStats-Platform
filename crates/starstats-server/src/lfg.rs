//! The Looking for Group board (social phase 4): the store.
//!
//! A post is a host's short-lived call for crew. Players ask to join and
//! the host accepts, declines or removes them. Posts expire (that is not
//! optional: a stale post is worse than none), can be closed by the host,
//! and can be taken down by a moderator after a report. Routes are in
//! `lfg_routes.rs`.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

/// A closed vocabulary stored as TEXT: `as_str` / `parse` round-trip, and
/// serde uses the same snake_case names.
macro_rules! vocab {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            // Not every vocabulary is offered as a list of choices.
            #[allow(dead_code)]
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s { $($text => Some($name::$variant),)+ _ => None }
            }
        }
    };
}

vocab!(
    /// What the group is for.
    LfgActivity {
        BountyHunting => "bounty_hunting",
        Mercenary => "mercenary",
        Fps => "fps",
        Mining => "mining",
        Salvage => "salvage",
        Hauling => "hauling",
        Exploration => "exploration",
        Racing => "racing",
        Medical => "medical",
        Piracy => "piracy",
        Social => "social",
        Other => "other",
    }
);

vocab!(
    /// Whether the group uses voice chat.
    LfgVoice {
        None => "none",
        Optional => "optional",
        Required => "required",
    }
);

vocab!(
    /// Where the host plays from, roughly, for timezone and latency.
    LfgRegion {
        Any => "any",
        Eu => "eu",
        Na => "na",
        Sa => "sa",
        Oce => "oce",
        Asia => "asia",
    }
);

vocab!(
    /// A player's standing on one post.
    MemberStatus {
        Requested => "requested",
        Accepted => "accepted",
        Declined => "declined",
        Left => "left",
        Removed => "removed",
    }
);

vocab!(
    LfgReportReason {
        Abuse => "abuse",
        Spam => "spam",
        IllegalContent => "illegal_content",
        Other => "other",
    }
);

vocab!(
    LfgReportStatus {
        Open => "open",
        Dismissed => "dismissed",
        PostRemoved => "post_removed",
        UserSuspended => "user_suspended",
    }
);

pub const LOCATION_MAX: usize = 64;
pub const SHIP_MAX: usize = 64;
pub const NOTE_MAX: usize = 200;
pub const CREW_MIN: i16 = 1;
pub const CREW_MAX: i16 = 30;
/// How long a post stays up, in minutes: 2 h unless the host says
/// otherwise, never more than 6 h.
pub const EXPIRY_DEFAULT_MINUTES: i64 = 120;
pub const EXPIRY_MIN_MINUTES: i64 = 15;
pub const EXPIRY_MAX_MINUTES: i64 = 360;
/// Posts a host can create in a day. A real host rarely makes more than a
/// few; a spammer is stopped quickly.
pub const POSTS_PER_DAY: i64 = 8;
/// Reports a player can file in a day, as for share reports.
pub const REPORTS_PER_DAY: i64 = 5;
pub const REPORT_DETAILS_MAX: usize = 500;
pub const RESOLUTION_NOTE_MAX: usize = 500;
/// How long an ended post (expired, closed or removed) is kept before it
/// is deleted, unless an open report still points at it.
pub const RETAIN_ENDED_DAYS: i64 = 30;

pub fn day() -> Duration {
    Duration::hours(24)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LfgPost {
    pub id: Uuid,
    pub host_handle: String,
    pub activity: LfgActivity,
    pub system: Option<String>,
    pub location: Option<String>,
    pub ship: Option<String>,
    pub crew_slots: i16,
    pub voice: LfgVoice,
    pub region: LfgRegion,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub removed_at: Option<DateTime<Utc>>,
    /// Accepted crew, not counting the host.
    pub crew_count: i64,
}

impl LfgPost {
    pub fn is_open(&self, now: DateTime<Utc>) -> bool {
        self.closed_at.is_none() && self.removed_at.is_none() && self.expires_at > now
    }
}

#[derive(Debug, Clone)]
pub struct NewLfgPost {
    pub host_handle: String,
    pub activity: LfgActivity,
    pub system: Option<String>,
    pub location: Option<String>,
    pub ship: Option<String>,
    pub crew_slots: i16,
    pub voice: LfgVoice,
    pub region: LfgRegion,
    pub note: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LfgMember {
    pub handle: String,
    pub status: MemberStatus,
    pub created_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LfgReport {
    pub id: Uuid,
    pub post_id: Uuid,
    pub reporter_handle: String,
    pub host_handle: String,
    pub reason: LfgReportReason,
    pub details: Option<String>,
    pub post_snapshot: serde_json::Value,
    pub status: LfgReportStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<String>,
    pub resolution_note: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LfgError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("stored value out of domain: {0}")]
    Domain(String),
    #[error("already asked or already crew")]
    AlreadyMember,
    #[error("removed from this group")]
    Removed,
    #[error("not found")]
    NotFound,
    #[error("report already resolved")]
    AlreadyResolved,
}

#[async_trait]
pub trait LfgStore: Send + Sync + 'static {
    async fn create_post(&self, post: NewLfgPost) -> Result<LfgPost, LfgError>;
    async fn get_post(&self, id: Uuid) -> Result<Option<LfgPost>, LfgError>;
    /// The host's post that is still open at `now`, if any.
    async fn open_post_for_host(
        &self,
        host: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<LfgPost>, LfgError>;
    async fn count_posts_since(&self, host: &str, since: DateTime<Utc>) -> Result<i64, LfgError>;
    /// Open posts at `now`, soonest to expire last, optionally filtered.
    async fn list_open(
        &self,
        now: DateTime<Utc>,
        activity: Option<LfgActivity>,
        system: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LfgPost>, LfgError>;
    /// `true` when an open post was closed.
    async fn close_post(&self, id: Uuid, now: DateTime<Utc>) -> Result<bool, LfgError>;
    async fn remove_post(
        &self,
        id: Uuid,
        by: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, LfgError>;

    async fn members(&self, post_id: Uuid) -> Result<Vec<LfgMember>, LfgError>;
    async fn member(&self, post_id: Uuid, handle: &str) -> Result<Option<LfgMember>, LfgError>;
    /// Ask to join. A player who left or was declined may ask again; one
    /// who was removed may not; one already asking or aboard gets
    /// `AlreadyMember`.
    async fn request_join(&self, post_id: Uuid, handle: &str) -> Result<LfgMember, LfgError>;
    async fn set_member_status(
        &self,
        post_id: Uuid,
        handle: &str,
        status: MemberStatus,
        now: DateTime<Utc>,
    ) -> Result<Option<LfgMember>, LfgError>;

    async fn create_report(
        &self,
        post: &LfgPost,
        reporter: &str,
        reason: LfgReportReason,
        details: Option<&str>,
    ) -> Result<LfgReport, LfgError>;
    async fn count_reports_since(
        &self,
        reporter: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, LfgError>;
    async fn list_reports(
        &self,
        status: Option<LfgReportStatus>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LfgReport>, LfgError>;
    async fn get_report(&self, id: Uuid) -> Result<Option<LfgReport>, LfgError>;
    /// Resolve an open report. `AlreadyResolved` if it is not open.
    async fn resolve_report(
        &self,
        id: Uuid,
        by: &str,
        outcome: LfgReportStatus,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<LfgReport, LfgError>;

    /// Delete posts that ended before `before` and have no open report.
    async fn purge_ended(&self, before: DateTime<Utc>) -> Result<u64, LfgError>;
}

// -- Postgres ----------------------------------------------------------------

pub struct PostgresLfgStore {
    pool: PgPool,
}

impl PostgresLfgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const POST_COLUMNS: &str = "p.id, p.host_handle, p.activity, p.system, p.location, p.ship, \
     p.crew_slots, p.voice, p.region, p.note, p.created_at, p.expires_at, p.closed_at, \
     p.removed_at, \
     (SELECT count(*) FROM lfg_members m WHERE m.post_id = p.id AND m.status = 'accepted') \
     AS crew_count";

type PostRow = (
    Uuid,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    i16,
    String,
    String,
    Option<String>,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    i64,
);

fn domain<T>(what: &str, v: &str, parsed: Option<T>) -> Result<T, LfgError> {
    parsed.ok_or_else(|| LfgError::Domain(format!("{what}={v}")))
}

fn post_from(r: PostRow) -> Result<LfgPost, LfgError> {
    Ok(LfgPost {
        id: r.0,
        host_handle: r.1,
        activity: domain("activity", &r.2, LfgActivity::parse(&r.2))?,
        system: r.3,
        location: r.4,
        ship: r.5,
        crew_slots: r.6,
        voice: domain("voice", &r.7, LfgVoice::parse(&r.7))?,
        region: domain("region", &r.8, LfgRegion::parse(&r.8))?,
        note: r.9,
        created_at: r.10,
        expires_at: r.11,
        closed_at: r.12,
        removed_at: r.13,
        crew_count: r.14,
    })
}

type MemberRow = (String, String, DateTime<Utc>, Option<DateTime<Utc>>);

fn member_from(r: MemberRow) -> Result<LfgMember, LfgError> {
    Ok(LfgMember {
        handle: r.0,
        status: domain("status", &r.1, MemberStatus::parse(&r.1))?,
        created_at: r.2,
        responded_at: r.3,
    })
}

const REPORT_COLUMNS: &str = "id, post_id, reporter_handle, host_handle, reason, details, \
     post_snapshot, status, created_at, resolved_at, resolved_by, resolution_note";

type ReportRow = (
    Uuid,
    Uuid,
    String,
    String,
    String,
    Option<String>,
    serde_json::Value,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
);

fn report_from(r: ReportRow) -> Result<LfgReport, LfgError> {
    Ok(LfgReport {
        id: r.0,
        post_id: r.1,
        reporter_handle: r.2,
        host_handle: r.3,
        reason: domain("reason", &r.4, LfgReportReason::parse(&r.4))?,
        details: r.5,
        post_snapshot: r.6,
        status: domain("status", &r.7, LfgReportStatus::parse(&r.7))?,
        created_at: r.8,
        resolved_at: r.9,
        resolved_by: r.10,
        resolution_note: r.11,
    })
}

#[async_trait]
impl LfgStore for PostgresLfgStore {
    async fn create_post(&self, post: NewLfgPost) -> Result<LfgPost, LfgError> {
        let (id,): (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO lfg_posts
                (host_handle, activity, system, location, ship, crew_slots, voice, region,
                 note, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id
            "#,
        )
        .bind(&post.host_handle)
        .bind(post.activity.as_str())
        .bind(&post.system)
        .bind(&post.location)
        .bind(&post.ship)
        .bind(post.crew_slots)
        .bind(post.voice.as_str())
        .bind(post.region.as_str())
        .bind(&post.note)
        .bind(post.expires_at)
        .fetch_one(&self.pool)
        .await?;
        self.get_post(id).await?.ok_or(LfgError::NotFound)
    }

    async fn get_post(&self, id: Uuid) -> Result<Option<LfgPost>, LfgError> {
        let row: Option<PostRow> = sqlx::query_as(&format!(
            "SELECT {POST_COLUMNS} FROM lfg_posts p WHERE p.id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(post_from).transpose()
    }

    async fn open_post_for_host(
        &self,
        host: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<LfgPost>, LfgError> {
        let row: Option<PostRow> = sqlx::query_as(&format!(
            "SELECT {POST_COLUMNS} FROM lfg_posts p
             WHERE lower(p.host_handle) = lower($1)
               AND p.closed_at IS NULL AND p.removed_at IS NULL AND p.expires_at > $2
             ORDER BY p.created_at DESC LIMIT 1"
        ))
        .bind(host)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        row.map(post_from).transpose()
    }

    async fn count_posts_since(&self, host: &str, since: DateTime<Utc>) -> Result<i64, LfgError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM lfg_posts WHERE lower(host_handle) = lower($1) AND created_at > $2",
        )
        .bind(host)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    async fn list_open(
        &self,
        now: DateTime<Utc>,
        activity: Option<LfgActivity>,
        system: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LfgPost>, LfgError> {
        let rows: Vec<PostRow> = sqlx::query_as(&format!(
            "SELECT {POST_COLUMNS} FROM lfg_posts p
             WHERE p.closed_at IS NULL AND p.removed_at IS NULL AND p.expires_at > $1
               AND ($2::text IS NULL OR p.activity = $2)
               AND ($3::text IS NULL OR lower(p.system) = lower($3))
             ORDER BY p.created_at DESC
             LIMIT $4"
        ))
        .bind(now)
        .bind(activity.map(|a| a.as_str()))
        .bind(system)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(post_from).collect()
    }

    async fn close_post(&self, id: Uuid, now: DateTime<Utc>) -> Result<bool, LfgError> {
        let res = sqlx::query(
            "UPDATE lfg_posts SET closed_at = $2
             WHERE id = $1 AND closed_at IS NULL AND removed_at IS NULL",
        )
        .bind(id)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn remove_post(
        &self,
        id: Uuid,
        by: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, LfgError> {
        let res = sqlx::query(
            "UPDATE lfg_posts SET removed_at = $2, removed_by = $3, removed_reason = $4
             WHERE id = $1 AND removed_at IS NULL",
        )
        .bind(id)
        .bind(now)
        .bind(by)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn members(&self, post_id: Uuid) -> Result<Vec<LfgMember>, LfgError> {
        let rows: Vec<MemberRow> = sqlx::query_as(
            "SELECT member_handle, status, created_at, responded_at FROM lfg_members
             WHERE post_id = $1 ORDER BY created_at",
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(member_from).collect()
    }

    async fn member(&self, post_id: Uuid, handle: &str) -> Result<Option<LfgMember>, LfgError> {
        let row: Option<MemberRow> = sqlx::query_as(
            "SELECT member_handle, status, created_at, responded_at FROM lfg_members
             WHERE post_id = $1 AND lower(member_handle) = lower($2)",
        )
        .bind(post_id)
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        row.map(member_from).transpose()
    }

    async fn request_join(&self, post_id: Uuid, handle: &str) -> Result<LfgMember, LfgError> {
        match self.member(post_id, handle).await? {
            Some(m) if m.status == MemberStatus::Removed => return Err(LfgError::Removed),
            Some(m) if matches!(m.status, MemberStatus::Requested | MemberStatus::Accepted) => {
                return Err(LfgError::AlreadyMember)
            }
            _ => {}
        }
        let row: MemberRow = sqlx::query_as(
            r#"
            INSERT INTO lfg_members (post_id, member_handle, status) VALUES ($1, $2, 'requested')
            ON CONFLICT (post_id, lower(member_handle))
            DO UPDATE SET status = 'requested', created_at = now(), responded_at = NULL
            WHERE lfg_members.status IN ('left', 'declined')
            RETURNING member_handle, status, created_at, responded_at
            "#,
        )
        .bind(post_id)
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?
        // A concurrent request or removal won the race.
        .ok_or(LfgError::AlreadyMember)?;
        member_from(row)
    }

    async fn set_member_status(
        &self,
        post_id: Uuid,
        handle: &str,
        status: MemberStatus,
        now: DateTime<Utc>,
    ) -> Result<Option<LfgMember>, LfgError> {
        let row: Option<MemberRow> = sqlx::query_as(
            "UPDATE lfg_members SET status = $3, responded_at = $4
             WHERE post_id = $1 AND lower(member_handle) = lower($2)
             RETURNING member_handle, status, created_at, responded_at",
        )
        .bind(post_id)
        .bind(handle)
        .bind(status.as_str())
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        row.map(member_from).transpose()
    }

    async fn create_report(
        &self,
        post: &LfgPost,
        reporter: &str,
        reason: LfgReportReason,
        details: Option<&str>,
    ) -> Result<LfgReport, LfgError> {
        let snapshot = serde_json::to_value(post).unwrap_or(serde_json::Value::Null);
        let row: ReportRow = sqlx::query_as(&format!(
            "INSERT INTO lfg_reports (post_id, reporter_handle, host_handle, reason, details, post_snapshot)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING {REPORT_COLUMNS}"
        ))
        .bind(post.id)
        .bind(reporter)
        .bind(&post.host_handle)
        .bind(reason.as_str())
        .bind(details)
        .bind(snapshot)
        .fetch_one(&self.pool)
        .await?;
        report_from(row)
    }

    async fn count_reports_since(
        &self,
        reporter: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, LfgError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM lfg_reports
             WHERE lower(reporter_handle) = lower($1) AND created_at > $2",
        )
        .bind(reporter)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    async fn list_reports(
        &self,
        status: Option<LfgReportStatus>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LfgReport>, LfgError> {
        let rows: Vec<ReportRow> = sqlx::query_as(&format!(
            "SELECT {REPORT_COLUMNS} FROM lfg_reports
             WHERE ($1::text IS NULL OR status = $1)
             ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        ))
        .bind(status.map(|s| s.as_str()))
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(report_from).collect()
    }

    async fn get_report(&self, id: Uuid) -> Result<Option<LfgReport>, LfgError> {
        let row: Option<ReportRow> = sqlx::query_as(&format!(
            "SELECT {REPORT_COLUMNS} FROM lfg_reports WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(report_from).transpose()
    }

    async fn resolve_report(
        &self,
        id: Uuid,
        by: &str,
        outcome: LfgReportStatus,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<LfgReport, LfgError> {
        let row: Option<ReportRow> = sqlx::query_as(&format!(
            "UPDATE lfg_reports
             SET status = $2, resolved_at = $3, resolved_by = $4, resolution_note = $5
             WHERE id = $1 AND status = 'open'
             RETURNING {REPORT_COLUMNS}"
        ))
        .bind(id)
        .bind(outcome.as_str())
        .bind(now)
        .bind(by)
        .bind(note)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(r) => report_from(r),
            None => match self.get_report(id).await? {
                Some(_) => Err(LfgError::AlreadyResolved),
                None => Err(LfgError::NotFound),
            },
        }
    }

    async fn purge_ended(&self, before: DateTime<Utc>) -> Result<u64, LfgError> {
        let res = sqlx::query(
            "DELETE FROM lfg_posts p
             WHERE COALESCE(p.removed_at, p.closed_at, p.expires_at) < $1
               AND NOT EXISTS (
                   SELECT 1 FROM lfg_reports r WHERE r.post_id = p.id AND r.status = 'open'
               )",
        )
        .bind(before)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }
}

// -- Memory (tests) -----------------------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    fn eq(a: &str, b: &str) -> bool {
        a.eq_ignore_ascii_case(b)
    }

    #[derive(Default)]
    struct Inner {
        posts: Vec<LfgPost>,
        members: Vec<(Uuid, LfgMember)>,
        reports: Vec<LfgReport>,
    }

    #[derive(Default)]
    pub struct MemoryLfgStore {
        inner: Mutex<Inner>,
    }

    impl MemoryLfgStore {
        pub fn new() -> Self {
            Self::default()
        }

        fn with_count(g: &Inner, mut p: LfgPost) -> LfgPost {
            p.crew_count = g
                .members
                .iter()
                .filter(|(id, m)| *id == p.id && m.status == MemberStatus::Accepted)
                .count() as i64;
            p
        }
    }

    #[async_trait]
    impl LfgStore for MemoryLfgStore {
        async fn create_post(&self, post: NewLfgPost) -> Result<LfgPost, LfgError> {
            let mut g = self.inner.lock().unwrap();
            let p = LfgPost {
                id: Uuid::now_v7(),
                host_handle: post.host_handle,
                activity: post.activity,
                system: post.system,
                location: post.location,
                ship: post.ship,
                crew_slots: post.crew_slots,
                voice: post.voice,
                region: post.region,
                note: post.note,
                created_at: Utc::now(),
                expires_at: post.expires_at,
                closed_at: None,
                removed_at: None,
                crew_count: 0,
            };
            g.posts.push(p.clone());
            Ok(p)
        }

        async fn get_post(&self, id: Uuid) -> Result<Option<LfgPost>, LfgError> {
            let g = self.inner.lock().unwrap();
            Ok(g.posts
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .map(|p| Self::with_count(&g, p)))
        }

        async fn open_post_for_host(
            &self,
            host: &str,
            now: DateTime<Utc>,
        ) -> Result<Option<LfgPost>, LfgError> {
            let g = self.inner.lock().unwrap();
            Ok(g.posts
                .iter()
                .rev()
                .find(|p| eq(&p.host_handle, host) && p.is_open(now))
                .cloned()
                .map(|p| Self::with_count(&g, p)))
        }

        async fn count_posts_since(
            &self,
            host: &str,
            since: DateTime<Utc>,
        ) -> Result<i64, LfgError> {
            let g = self.inner.lock().unwrap();
            Ok(g.posts
                .iter()
                .filter(|p| eq(&p.host_handle, host) && p.created_at > since)
                .count() as i64)
        }

        async fn list_open(
            &self,
            now: DateTime<Utc>,
            activity: Option<LfgActivity>,
            system: Option<&str>,
            limit: i64,
        ) -> Result<Vec<LfgPost>, LfgError> {
            let g = self.inner.lock().unwrap();
            let mut v: Vec<LfgPost> = g
                .posts
                .iter()
                .filter(|p| p.is_open(now))
                .filter(|p| activity.is_none_or(|a| p.activity == a))
                .filter(|p| system.is_none_or(|s| p.system.as_deref().is_some_and(|ps| eq(ps, s))))
                .cloned()
                .map(|p| Self::with_count(&g, p))
                .collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.created_at));
            v.truncate(limit as usize);
            Ok(v)
        }

        async fn close_post(&self, id: Uuid, now: DateTime<Utc>) -> Result<bool, LfgError> {
            let mut g = self.inner.lock().unwrap();
            match g
                .posts
                .iter_mut()
                .find(|p| p.id == id && p.closed_at.is_none() && p.removed_at.is_none())
            {
                Some(p) => {
                    p.closed_at = Some(now);
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        async fn remove_post(
            &self,
            id: Uuid,
            _by: &str,
            _reason: &str,
            now: DateTime<Utc>,
        ) -> Result<bool, LfgError> {
            let mut g = self.inner.lock().unwrap();
            match g
                .posts
                .iter_mut()
                .find(|p| p.id == id && p.removed_at.is_none())
            {
                Some(p) => {
                    p.removed_at = Some(now);
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        async fn members(&self, post_id: Uuid) -> Result<Vec<LfgMember>, LfgError> {
            let g = self.inner.lock().unwrap();
            Ok(g.members
                .iter()
                .filter(|(id, _)| *id == post_id)
                .map(|(_, m)| m.clone())
                .collect())
        }

        async fn member(&self, post_id: Uuid, handle: &str) -> Result<Option<LfgMember>, LfgError> {
            let g = self.inner.lock().unwrap();
            Ok(g.members
                .iter()
                .find(|(id, m)| *id == post_id && eq(&m.handle, handle))
                .map(|(_, m)| m.clone()))
        }

        async fn request_join(&self, post_id: Uuid, handle: &str) -> Result<LfgMember, LfgError> {
            let mut g = self.inner.lock().unwrap();
            if let Some((_, m)) = g
                .members
                .iter_mut()
                .find(|(id, m)| *id == post_id && eq(&m.handle, handle))
            {
                return match m.status {
                    MemberStatus::Removed => Err(LfgError::Removed),
                    MemberStatus::Requested | MemberStatus::Accepted => {
                        Err(LfgError::AlreadyMember)
                    }
                    MemberStatus::Left | MemberStatus::Declined => {
                        m.status = MemberStatus::Requested;
                        m.created_at = Utc::now();
                        m.responded_at = None;
                        Ok(m.clone())
                    }
                };
            }
            let m = LfgMember {
                handle: handle.to_string(),
                status: MemberStatus::Requested,
                created_at: Utc::now(),
                responded_at: None,
            };
            g.members.push((post_id, m.clone()));
            Ok(m)
        }

        async fn set_member_status(
            &self,
            post_id: Uuid,
            handle: &str,
            status: MemberStatus,
            now: DateTime<Utc>,
        ) -> Result<Option<LfgMember>, LfgError> {
            let mut g = self.inner.lock().unwrap();
            Ok(g.members
                .iter_mut()
                .find(|(id, m)| *id == post_id && eq(&m.handle, handle))
                .map(|(_, m)| {
                    m.status = status;
                    m.responded_at = Some(now);
                    m.clone()
                }))
        }

        async fn create_report(
            &self,
            post: &LfgPost,
            reporter: &str,
            reason: LfgReportReason,
            details: Option<&str>,
        ) -> Result<LfgReport, LfgError> {
            let mut g = self.inner.lock().unwrap();
            let r = LfgReport {
                id: Uuid::now_v7(),
                post_id: post.id,
                reporter_handle: reporter.to_string(),
                host_handle: post.host_handle.clone(),
                reason,
                details: details.map(str::to_string),
                post_snapshot: serde_json::to_value(post).unwrap(),
                status: LfgReportStatus::Open,
                created_at: Utc::now(),
                resolved_at: None,
                resolved_by: None,
                resolution_note: None,
            };
            g.reports.push(r.clone());
            Ok(r)
        }

        async fn count_reports_since(
            &self,
            reporter: &str,
            since: DateTime<Utc>,
        ) -> Result<i64, LfgError> {
            let g = self.inner.lock().unwrap();
            Ok(g.reports
                .iter()
                .filter(|r| eq(&r.reporter_handle, reporter) && r.created_at > since)
                .count() as i64)
        }

        async fn list_reports(
            &self,
            status: Option<LfgReportStatus>,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<LfgReport>, LfgError> {
            let g = self.inner.lock().unwrap();
            let mut v: Vec<LfgReport> = g
                .reports
                .iter()
                .filter(|r| status.is_none_or(|s| r.status == s))
                .cloned()
                .collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.created_at));
            Ok(v.into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect())
        }

        async fn get_report(&self, id: Uuid) -> Result<Option<LfgReport>, LfgError> {
            let g = self.inner.lock().unwrap();
            Ok(g.reports.iter().find(|r| r.id == id).cloned())
        }

        async fn resolve_report(
            &self,
            id: Uuid,
            by: &str,
            outcome: LfgReportStatus,
            note: Option<&str>,
            now: DateTime<Utc>,
        ) -> Result<LfgReport, LfgError> {
            let mut g = self.inner.lock().unwrap();
            let r = g
                .reports
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or(LfgError::NotFound)?;
            if r.status != LfgReportStatus::Open {
                return Err(LfgError::AlreadyResolved);
            }
            r.status = outcome;
            r.resolved_at = Some(now);
            r.resolved_by = Some(by.to_string());
            r.resolution_note = note.map(str::to_string);
            Ok(r.clone())
        }

        async fn purge_ended(&self, before: DateTime<Utc>) -> Result<u64, LfgError> {
            let mut g = self.inner.lock().unwrap();
            let open_reported: Vec<Uuid> = g
                .reports
                .iter()
                .filter(|r| r.status == LfgReportStatus::Open)
                .map(|r| r.post_id)
                .collect();
            let n = g.posts.len();
            g.posts.retain(|p| {
                let ended = p.removed_at.or(p.closed_at).unwrap_or(p.expires_at);
                !(ended < before && !open_reported.contains(&p.id))
            });
            let kept: Vec<Uuid> = g.posts.iter().map(|p| p.id).collect();
            g.members.retain(|(id, _)| kept.contains(id));
            Ok((n - g.posts.len()) as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryLfgStore;
    use super::*;

    fn new_post(host: &str, minutes: i64) -> NewLfgPost {
        NewLfgPost {
            host_handle: host.into(),
            activity: LfgActivity::Mining,
            system: Some("Stanton".into()),
            location: None,
            ship: Some("Prospector".into()),
            crew_slots: 2,
            voice: LfgVoice::Optional,
            region: LfgRegion::Eu,
            note: None,
            expires_at: Utc::now() + Duration::minutes(minutes),
        }
    }

    #[test]
    fn vocabularies_round_trip() {
        for a in LfgActivity::ALL {
            assert_eq!(LfgActivity::parse(a.as_str()), Some(*a));
        }
        for s in LfgReportStatus::ALL {
            assert_eq!(LfgReportStatus::parse(s.as_str()), Some(*s));
        }
        assert_eq!(LfgRegion::parse("mars"), None);
        assert_eq!(
            serde_json::to_value(LfgActivity::BountyHunting).unwrap(),
            "bounty_hunting"
        );
    }

    #[tokio::test]
    async fn only_open_posts_are_listed() {
        let s = MemoryLfgStore::new();
        let open = s.create_post(new_post("Alice", 60)).await.unwrap();
        let expired = s.create_post(new_post("Bob", -1)).await.unwrap();
        let closed = s.create_post(new_post("Carol", 60)).await.unwrap();
        s.close_post(closed.id, Utc::now()).await.unwrap();
        let ids: Vec<Uuid> = s
            .list_open(Utc::now(), None, None, 50)
            .await
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, vec![open.id]);
        assert!(!ids.contains(&expired.id));
        assert!(s
            .list_open(Utc::now(), Some(LfgActivity::Racing), None, 50)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            s.list_open(Utc::now(), None, Some("stanton"), 50)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn joining_follows_the_member_states() {
        let s = MemoryLfgStore::new();
        let p = s.create_post(new_post("Alice", 60)).await.unwrap();
        s.request_join(p.id, "Bob").await.unwrap();
        assert!(matches!(
            s.request_join(p.id, "BOB").await,
            Err(LfgError::AlreadyMember)
        ));
        s.set_member_status(p.id, "bob", MemberStatus::Accepted, Utc::now())
            .await
            .unwrap();
        assert_eq!(s.get_post(p.id).await.unwrap().unwrap().crew_count, 1);
        s.set_member_status(p.id, "bob", MemberStatus::Left, Utc::now())
            .await
            .unwrap();
        assert_eq!(
            s.request_join(p.id, "Bob").await.unwrap().status,
            MemberStatus::Requested,
            "a player who left may ask again"
        );
        s.set_member_status(p.id, "bob", MemberStatus::Removed, Utc::now())
            .await
            .unwrap();
        assert!(matches!(
            s.request_join(p.id, "Bob").await,
            Err(LfgError::Removed)
        ));
    }

    #[tokio::test]
    async fn a_report_resolves_once_and_keeps_its_post() {
        let s = MemoryLfgStore::new();
        let p = s
            .create_post(new_post("Alice", -60 * 24 * 40))
            .await
            .unwrap();
        let r = s
            .create_report(&p, "Bob", LfgReportReason::Spam, Some("ads"))
            .await
            .unwrap();
        assert_eq!(r.post_snapshot["host_handle"], "Alice");
        // Ended long ago, but an open report keeps it.
        assert_eq!(s.purge_ended(Utc::now()).await.unwrap(), 0);
        s.resolve_report(r.id, "mod", LfgReportStatus::Dismissed, None, Utc::now())
            .await
            .unwrap();
        assert!(matches!(
            s.resolve_report(r.id, "mod", LfgReportStatus::PostRemoved, None, Utc::now())
                .await,
            Err(LfgError::AlreadyResolved)
        ));
        assert_eq!(s.purge_ended(Utc::now()).await.unwrap(), 1);
        assert!(
            s.get_report(r.id).await.unwrap().is_some(),
            "the report outlives it"
        );
    }

    /// The SQL paths the memory store cannot vouch for: the join upsert
    /// that only reopens a left or declined row, the crew count, and the
    /// retention purge. Skipped without STARSTATS_TEST_DATABASE_URL.
    #[tokio::test]
    async fn postgres_lfg_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres LFG test");
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
        sqlx::query("DELETE FROM lfg_reports WHERE lower(host_handle) = 'lfgprobehost'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM lfg_posts WHERE lower(host_handle) = 'lfgprobehost'")
            .execute(&pool)
            .await
            .unwrap();
        let s = PostgresLfgStore::new(pool.clone());
        let p = s.create_post(new_post("LfgProbeHost", 60)).await.unwrap();
        assert_eq!(p.activity, LfgActivity::Mining);
        assert!(s
            .open_post_for_host("lfgprobehost", Utc::now())
            .await
            .unwrap()
            .is_some());

        s.request_join(p.id, "LfgProbeBob").await.unwrap();
        assert!(matches!(
            s.request_join(p.id, "lfgprobebob").await,
            Err(LfgError::AlreadyMember)
        ));
        s.set_member_status(p.id, "LFGPROBEBOB", MemberStatus::Accepted, Utc::now())
            .await
            .unwrap();
        assert_eq!(s.get_post(p.id).await.unwrap().unwrap().crew_count, 1);
        s.set_member_status(p.id, "lfgprobebob", MemberStatus::Declined, Utc::now())
            .await
            .unwrap();
        assert_eq!(
            s.request_join(p.id, "LfgProbeBob").await.unwrap().status,
            MemberStatus::Requested
        );
        s.set_member_status(p.id, "lfgprobebob", MemberStatus::Removed, Utc::now())
            .await
            .unwrap();
        assert!(matches!(
            s.request_join(p.id, "LfgProbeBob").await,
            Err(LfgError::Removed)
        ));

        let r = s
            .create_report(&p, "LfgProbeBob", LfgReportReason::Abuse, None)
            .await
            .unwrap();
        s.close_post(p.id, Utc::now()).await.unwrap();
        let later = Utc::now() + Duration::days(RETAIN_ENDED_DAYS + 1);
        assert_eq!(
            s.purge_ended(later).await.unwrap(),
            0,
            "an open report keeps it"
        );
        s.resolve_report(r.id, "mod", LfgReportStatus::Dismissed, None, Utc::now())
            .await
            .unwrap();
        assert!(s.purge_ended(later).await.unwrap() >= 1);
        assert!(s.get_post(p.id).await.unwrap().is_none());
        assert!(
            s.members(p.id).await.unwrap().is_empty(),
            "crew rows go with it"
        );
        sqlx::query("DELETE FROM lfg_reports WHERE id = $1")
            .bind(r.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
