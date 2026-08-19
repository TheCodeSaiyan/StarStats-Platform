//! Community-curated parser-rule submissions.
//!
//! Three concerns live here:
//!  1. The data store (CRUD on the `submissions`, `submission_votes`,
//!     `submission_flags` tables).
//!  2. Per-user "once per item" semantics enforced via the composite
//!     primary keys — the store exposes idempotent `vote` / `flag`
//!     methods that return whether a write actually happened.
//!  3. Auto-escalation: when distinct flag count crosses
//!     [`AUTO_FLAG_THRESHOLD`] the submission's status flips from
//!     `review` to `flagged` so a moderator (future wave) can review.
//!
//! HTTP handlers live in `submission_routes`. The store trait is the
//! only thing they touch, so the route tests use [`MemorySubmissionStore`]
//! and the production wiring uses [`PostgresSubmissionStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Distinct flaggers required to auto-route a submission to moderator
/// review. Tuned conservatively: three independent users marking the
/// same submission as "this is wrong" is a strong-enough signal that
/// we shouldn't keep advertising it to the wider voter pool, but low
/// enough that a determined troll alone can't silence a submission.
pub const AUTO_FLAG_THRESHOLD: i64 = 3;

/// Hard caps on the free-form text fields. Wide enough for real
/// proposals (a short README-paragraph rationale), narrow enough that
/// a single submission can't wedge a row beyond TOAST size.
pub const PATTERN_MAX_LEN: usize = 512;
pub const LABEL_MAX_LEN: usize = 64;
pub const DESCRIPTION_MAX_LEN: usize = 2_000;
pub const SAMPLE_LINE_MAX_LEN: usize = 2_000;
pub const FLAG_REASON_MAX_LEN: usize = 1_000;

/// System account that owns anonymously-promoted community submissions.
/// Seeded by migration 0054. Tray-originated promotions attribute here
/// (or to the tray user's own account, if they opted into attribution)
/// so `submissions.submitter_id` stays NOT NULL even when the tray
/// submission carried no user identity.
///
/// Used by the admin publish handler
/// (`admin_parser_submissions::publish_to_community`) to attribute an
/// anonymously-promoted shape.
///
/// The row's stored `claimed_handle` is the unclaimable `community.system`
/// (see migration 0054) -- users never see that string. [`display_handle`]
/// maps this UUID to the display label `community` on every read path.
pub const COMMUNITY_USER_ID: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_000b_0700);

/// Resolve the handle shown to users for a submission's owner.
///
/// The seeded community system account ([`COMMUNITY_USER_ID`]) always
/// displays as `community`, no matter what unclaimable raw handle
/// (`community.system`) backs it in the `users` table. This is the
/// display-side counterpart to the collision-proof unclaimable seed
/// handle in migration 0054: we seed an unclaimable handle so the
/// `ON CONFLICT DO NOTHING` insert can never be blocked by a real user,
/// and remap the display here so the public list/detail and the
/// moderator queue both show `community`.
///
/// Every read path that materialises a [`Submission`] funnels the
/// joined/stored handle through this one function (DRY), so the mapping
/// can never drift between surfaces. For any other owner the raw handle
/// passes through unchanged.
pub fn display_handle(submitter_id: Uuid, raw_handle: String) -> String {
    if submitter_id == COMMUNITY_USER_ID {
        "community".to_string()
    } else {
        raw_handle
    }
}

/// One submission row plus the materialised counts the list/detail
/// views need. The counts are derived (not stored on the table itself)
/// so a stale read is impossible — we always recompute when surfacing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    pub id: Uuid,
    pub submitter_id: Uuid,
    pub submitter_handle: String,
    pub pattern: String,
    pub proposed_label: String,
    pub description: String,
    pub sample_line: String,
    pub log_source: String,
    pub status: SubmissionStatus,
    pub rejection_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub vote_count: i64,
    pub flag_count: i64,
}

/// Lifecycle states a submission can occupy. Mirrors the CHECK
/// constraint in migration 0016. We keep the variant set tight; new
/// states require a migration + variant + handler update so we notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStatus {
    Review,
    Accepted,
    Shipped,
    Rejected,
    Withdrawn,
    Flagged,
}

impl SubmissionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Accepted => "accepted",
            Self::Shipped => "shipped",
            Self::Rejected => "rejected",
            Self::Withdrawn => "withdrawn",
            Self::Flagged => "flagged",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "review" => Self::Review,
            "accepted" => Self::Accepted,
            "shipped" => Self::Shipped,
            "rejected" => Self::Rejected,
            "withdrawn" => Self::Withdrawn,
            "flagged" => Self::Flagged,
            _ => return None,
        })
    }
}

/// Outcome of a `vote` or `flag` write — distinguishes "first time"
/// from "already there" without a second round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Inserted,
    AlreadyExists,
}

/// Fields the API accepts when creating a submission. The store
/// validates length only; semantic validation (label slug shape) lives
/// at the route layer because it shares a regex with `event_type`.
#[derive(Debug, Clone)]
pub struct NewSubmission<'a> {
    pub submitter_id: Uuid,
    pub pattern: &'a str,
    pub proposed_label: &'a str,
    pub description: &'a str,
    pub sample_line: &'a str,
    pub log_source: &'a str,
}

/// Fields needed to promote a tray `parser_submissions` shape into the
/// public community `submissions` queue (Task 5's job). Distinct from
/// [`NewSubmission`] in that the submitter's handle is supplied
/// explicitly (the community account's log label, or the tray user's
/// attributed handle) rather than being looked up via the `users`
/// join, and the row carries `source_shape_hash` so re-publishing the
/// same shape is a no-op instead of a duplicate row.
#[derive(Debug, Clone)]
pub struct PromotedSubmission<'a> {
    pub submitter_id: Uuid,
    /// Not persisted as a column -- `submissions` has no
    /// `submitter_handle` field; the display handle is always resolved
    /// via the `users` join on read, and `submitter_id` alone drives
    /// that. This field feeds the publish handler's promotion log line.
    pub submitter_handle: &'a str,
    pub pattern: &'a str,
    pub proposed_label: &'a str,
    pub description: &'a str,
    pub sample_line: &'a str,
    pub log_source: &'a str,
    pub source_shape_hash: &'a str,
}

/// Sort order for the list endpoint. Maps to a fixed, whitelisted
/// `ORDER BY` fragment — never interpolates user input — so it is
/// injection-safe even though it's pushed as a literal into the query
/// builder. `Newest` (most-recently-updated first) is the default and
/// matches the prior hardcoded ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubmissionSort {
    #[default]
    Newest,
    Oldest,
    MostVotes,
}

impl SubmissionSort {
    /// Parse the `?sort=` query value. Returns `None` for unknown
    /// values so the route layer can 400 rather than silently
    /// defaulting.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "newest" => Some(Self::Newest),
            "oldest" => Some(Self::Oldest),
            "votes" => Some(Self::MostVotes),
            _ => None,
        }
    }

    /// Static `ORDER BY` clause (leading space included). All branches
    /// are compile-time literals — no user data reaches the SQL.
    fn order_by(self) -> &'static str {
        match self {
            Self::Newest => " ORDER BY s.updated_at DESC, s.id DESC",
            Self::Oldest => " ORDER BY s.updated_at ASC, s.id ASC",
            Self::MostVotes => " ORDER BY vote_count DESC, s.updated_at DESC, s.id DESC",
        }
    }
}

/// Filter for the list endpoint. `None` lifts the filter; specific
/// statuses page just within that bucket.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubmissionFilter {
    pub status: Option<SubmissionStatus>,
    pub mine_only: bool,
    pub sort: SubmissionSort,
}

/// Filter for the moderator queue. Only "actionable" states surface here;
/// `accepted`, `shipped`, `rejected`, and `withdrawn` are intentionally
/// excluded -- moderators don't need to see them in their work queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminQueueFilter {
    /// Only `status = 'review'`.
    Review,
    /// Only `status = 'flagged'`.
    Flagged,
    /// Both `review` and `flagged` (NOT accepted/shipped/rejected/withdrawn).
    All,
}

/// Outcome of an admin lifecycle transition. `was_changed=false` is
/// returned for idempotent no-ops (e.g. accepting an already-accepted
/// submission); the route layer uses that flag to skip writing an
/// audit row when the state didn't actually move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionTransition {
    pub id: Uuid,
    pub previous_status: String,
    pub new_status: String,
    pub was_changed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SubmissionError {
    #[error("submission not found")]
    NotFound,
    #[error("only the submitter can withdraw their own submission")]
    Forbidden,
    #[error("submission can only be withdrawn while in review")]
    BadState,
    #[error("illegal status transition from {from} to {to}")]
    IllegalTransition { from: String, to: String },
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait SubmissionStore: Send + Sync + 'static {
    async fn create(&self, new: NewSubmission<'_>) -> Result<Submission, SubmissionError>;

    async fn find_by_id(
        &self,
        id: Uuid,
        viewer_id: Option<Uuid>,
    ) -> Result<Option<SubmissionWithViewer>, SubmissionError>;

    async fn list(
        &self,
        filter: SubmissionFilter,
        viewer_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SubmissionWithViewer>, SubmissionError>;

    /// Idempotent: same user voting twice is a no-op and returns
    /// [`WriteOutcome::AlreadyExists`].
    async fn vote(
        &self,
        submission_id: Uuid,
        user_id: Uuid,
    ) -> Result<WriteOutcome, SubmissionError>;

    /// Removes a vote if the user had one; returns whether a row was
    /// removed. Used by the toggle behaviour on the detail page.
    async fn unvote(
        &self,
        submission_id: Uuid,
        user_id: Uuid,
    ) -> Result<WriteOutcome, SubmissionError>;

    /// Insert a flag and, if the distinct flagger count crosses
    /// [`AUTO_FLAG_THRESHOLD`], flip the submission to `flagged`.
    /// Returns the post-write flag count + whether the auto-escalation
    /// fired so the route layer can surface that fact in the response.
    async fn flag(
        &self,
        submission_id: Uuid,
        user_id: Uuid,
        reason: Option<&str>,
    ) -> Result<FlagOutcome, SubmissionError>;

    /// Submitter-only: flip status to `withdrawn`. The route layer
    /// owns the auth check (matching submitter_id == caller) — the
    /// store enforces the "must be in review" precondition.
    async fn withdraw(
        &self,
        submission_id: Uuid,
        caller_id: Uuid,
    ) -> Result<Submission, SubmissionError>;

    /// Moderator action: move `review`/`flagged` -> `accepted`. Idempotent:
    /// accepting an already-accepted row returns
    /// `was_changed=false` and no rows are touched. Anything outside
    /// `{review, flagged, accepted}` returns `IllegalTransition`.
    /// `moderator_id` is the staff user performing the action; stores
    /// don't currently persist it themselves (the audit log carries the
    /// actor), but the parameter is reserved for future moderator-id
    /// columns.
    async fn accept_submission(
        &self,
        submission_id: Uuid,
        moderator_id: Uuid,
    ) -> Result<SubmissionTransition, SubmissionError>;

    /// Moderator action: move `review`/`flagged` -> `rejected`, storing
    /// `reason` in `rejection_reason`. Idempotent: rejecting an
    /// already-rejected row returns `was_changed=false` and never
    /// rewrites the stored reason (treat differing reasons as a no-op).
    async fn reject_submission(
        &self,
        submission_id: Uuid,
        moderator_id: Uuid,
        reason: &str,
    ) -> Result<SubmissionTransition, SubmissionError>;

    /// Moderator action: move `flagged` -> `review` so users can
    /// re-vote / re-flag. Idempotent on `review` (returns
    /// `was_changed=false`). Other states return `IllegalTransition`.
    async fn dismiss_flag(
        &self,
        submission_id: Uuid,
        moderator_id: Uuid,
    ) -> Result<SubmissionTransition, SubmissionError>;

    /// Paged list of moderator-actionable submissions. Mirrors `list`
    /// but skips the per-viewer projection and applies the
    /// admin-specific status filter (`review`/`flagged`/both).
    async fn list_admin_queue(
        &self,
        filter: AdminQueueFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Submission>, SubmissionError>;

    // `find_by_source_shape` still has no call site outside this module's
    // tests yet; `#[allow(dead_code)]` keeps `-D warnings` clean until a
    // caller lands. `mark_shipped_by_source` IS now called, by the admin
    // parser-submissions PATCH handler's `rule_written` transition.
    // (`create_promoted` is called by the admin publish handler.)

    /// Promote a tray shape into the community queue. Idempotent on
    /// `source_shape_hash`: re-promoting the same shape returns the
    /// existing row rather than inserting a duplicate (a shape can be
    /// republished across parser restarts / repeated syncs without
    /// creating a second submission).
    async fn create_promoted(
        &self,
        p: PromotedSubmission<'_>,
    ) -> Result<Submission, SubmissionError>;

    /// Look up the community submission linked to a tray shape, if any.
    #[allow(dead_code)]
    async fn find_by_source_shape(
        &self,
        shape: &str,
    ) -> Result<Option<Submission>, SubmissionError>;

    /// Advance the submission linked to `shape` to `shipped`. A no-op
    /// (not an error) if no submission is linked to that shape.
    async fn mark_shipped_by_source(&self, shape: &str) -> Result<(), SubmissionError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionWithViewer {
    pub submission: Submission,
    pub viewer_voted: bool,
    pub viewer_flagged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagOutcome {
    pub write: WriteOutcome,
    pub flag_count: i64,
    /// True iff this flag's insert caused the status flip to
    /// `flagged`. False either because we were already past the
    /// threshold, or because we're still under it.
    pub escalated: bool,
}

// -- Postgres impl ---------------------------------------------------

pub struct PostgresSubmissionStore {
    pool: PgPool,
}

impl PostgresSubmissionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SubmissionStore for PostgresSubmissionStore {
    async fn create(&self, new: NewSubmission<'_>) -> Result<Submission, SubmissionError> {
        let id = Uuid::now_v7();
        let row: (DateTime<Utc>, DateTime<Utc>, String) = sqlx::query_as(
            "INSERT INTO submissions
                (id, submitter_id, pattern, proposed_label, description, sample_line, log_source)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING created_at, updated_at, status",
        )
        .bind(id)
        .bind(new.submitter_id)
        .bind(new.pattern)
        .bind(new.proposed_label)
        .bind(new.description)
        .bind(new.sample_line)
        .bind(new.log_source)
        .fetch_one(&self.pool)
        .await?;

        let submitter_handle = lookup_handle(&self.pool, new.submitter_id).await?;

        Ok(Submission {
            id,
            submitter_id: new.submitter_id,
            submitter_handle: display_handle(new.submitter_id, submitter_handle),
            pattern: new.pattern.to_string(),
            proposed_label: new.proposed_label.to_string(),
            description: new.description.to_string(),
            sample_line: new.sample_line.to_string(),
            log_source: new.log_source.to_string(),
            status: SubmissionStatus::parse(&row.2).unwrap_or(SubmissionStatus::Review),
            rejection_reason: None,
            created_at: row.0,
            updated_at: row.1,
            vote_count: 0,
            flag_count: 0,
        })
    }

    async fn find_by_id(
        &self,
        id: Uuid,
        viewer_id: Option<Uuid>,
    ) -> Result<Option<SubmissionWithViewer>, SubmissionError> {
        let mut rows = list_internal(
            &self.pool,
            FindSpec::ById(id),
            viewer_id,
            1,
            0,
            SubmissionSort::Newest,
        )
        .await?;
        Ok(rows.pop())
    }

    async fn list(
        &self,
        filter: SubmissionFilter,
        viewer_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SubmissionWithViewer>, SubmissionError> {
        list_internal(
            &self.pool,
            FindSpec::Filter(filter),
            viewer_id,
            limit,
            offset,
            filter.sort,
        )
        .await
    }

    async fn vote(
        &self,
        submission_id: Uuid,
        user_id: Uuid,
    ) -> Result<WriteOutcome, SubmissionError> {
        let inserted: Option<(Uuid,)> = sqlx::query_as(
            "INSERT INTO submission_votes (submission_id, user_id)
             VALUES ($1, $2)
             ON CONFLICT DO NOTHING
             RETURNING submission_id",
        )
        .bind(submission_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        if inserted.is_some() {
            // Touch updated_at so the list view's "recent activity"
            // ordering reflects votes too. Cheap — single UPDATE.
            sqlx::query("UPDATE submissions SET updated_at = NOW() WHERE id = $1")
                .bind(submission_id)
                .execute(&self.pool)
                .await?;
            Ok(WriteOutcome::Inserted)
        } else {
            Ok(WriteOutcome::AlreadyExists)
        }
    }

    async fn unvote(
        &self,
        submission_id: Uuid,
        user_id: Uuid,
    ) -> Result<WriteOutcome, SubmissionError> {
        let res = sqlx::query(
            "DELETE FROM submission_votes
             WHERE submission_id = $1 AND user_id = $2",
        )
        .bind(submission_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(if res.rows_affected() > 0 {
            WriteOutcome::Inserted
        } else {
            WriteOutcome::AlreadyExists
        })
    }

    async fn flag(
        &self,
        submission_id: Uuid,
        user_id: Uuid,
        reason: Option<&str>,
    ) -> Result<FlagOutcome, SubmissionError> {
        let mut tx = self.pool.begin().await?;

        // Snapshot the current status so we can detect the transition
        // edge — we only want `escalated=true` on the *first* flip.
        let prior: Option<(String,)> =
            sqlx::query_as("SELECT status FROM submissions WHERE id = $1 FOR UPDATE")
                .bind(submission_id)
                .fetch_optional(&mut *tx)
                .await?;
        let prior_status = prior
            .as_ref()
            .and_then(|s| SubmissionStatus::parse(&s.0))
            .ok_or(SubmissionError::NotFound)?;

        let inserted: Option<(Uuid,)> = sqlx::query_as(
            "INSERT INTO submission_flags (submission_id, user_id, reason)
             VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING
             RETURNING submission_id",
        )
        .bind(submission_id)
        .bind(user_id)
        .bind(reason)
        .fetch_optional(&mut *tx)
        .await?;

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM submission_flags WHERE submission_id = $1",
        )
        .bind(submission_id)
        .fetch_one(&mut *tx)
        .await?;

        let escalated = inserted.is_some()
            && prior_status == SubmissionStatus::Review
            && count >= AUTO_FLAG_THRESHOLD;

        if escalated {
            sqlx::query(
                "UPDATE submissions
                 SET status = 'flagged', updated_at = NOW()
                 WHERE id = $1",
            )
            .bind(submission_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        Ok(FlagOutcome {
            write: if inserted.is_some() {
                WriteOutcome::Inserted
            } else {
                WriteOutcome::AlreadyExists
            },
            flag_count: count,
            escalated,
        })
    }

    async fn withdraw(
        &self,
        submission_id: Uuid,
        caller_id: Uuid,
    ) -> Result<Submission, SubmissionError> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(Uuid, String)> =
            sqlx::query_as("SELECT submitter_id, status FROM submissions WHERE id = $1 FOR UPDATE")
                .bind(submission_id)
                .fetch_optional(&mut *tx)
                .await?;
        let (submitter_id, status) = row.ok_or(SubmissionError::NotFound)?;
        if submitter_id != caller_id {
            return Err(SubmissionError::Forbidden);
        }
        if SubmissionStatus::parse(&status) != Some(SubmissionStatus::Review) {
            return Err(SubmissionError::BadState);
        }

        sqlx::query(
            "UPDATE submissions
             SET status = 'withdrawn', updated_at = NOW()
             WHERE id = $1",
        )
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        // Re-read with the materialised counts so the response carries
        // the same shape as create / list.
        let mut rows = list_internal(
            &self.pool,
            FindSpec::ById(submission_id),
            None,
            1,
            0,
            SubmissionSort::Newest,
        )
        .await?;
        rows.pop()
            .map(|s| s.submission)
            .ok_or(SubmissionError::NotFound)
    }

    async fn accept_submission(
        &self,
        submission_id: Uuid,
        _moderator_id: Uuid,
    ) -> Result<SubmissionTransition, SubmissionError> {
        admin_transition(
            &self.pool,
            submission_id,
            SubmissionStatus::Accepted,
            None,
            &[
                SubmissionStatus::Review,
                SubmissionStatus::Flagged,
                SubmissionStatus::Accepted,
            ],
        )
        .await
    }

    async fn reject_submission(
        &self,
        submission_id: Uuid,
        _moderator_id: Uuid,
        reason: &str,
    ) -> Result<SubmissionTransition, SubmissionError> {
        admin_transition(
            &self.pool,
            submission_id,
            SubmissionStatus::Rejected,
            Some(reason),
            &[
                SubmissionStatus::Review,
                SubmissionStatus::Flagged,
                SubmissionStatus::Rejected,
            ],
        )
        .await
    }

    async fn dismiss_flag(
        &self,
        submission_id: Uuid,
        _moderator_id: Uuid,
    ) -> Result<SubmissionTransition, SubmissionError> {
        admin_transition(
            &self.pool,
            submission_id,
            SubmissionStatus::Review,
            None,
            &[SubmissionStatus::Flagged, SubmissionStatus::Review],
        )
        .await
    }

    async fn list_admin_queue(
        &self,
        filter: AdminQueueFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Submission>, SubmissionError> {
        let statuses: &[&str] = match filter {
            AdminQueueFilter::Review => &["review"],
            AdminQueueFilter::Flagged => &["flagged"],
            AdminQueueFilter::All => &["review", "flagged"],
        };
        // No viewer projection: the moderator queue is a moderator
        // tool, not a personalised view. Reuse the materialised-count
        // SELECT shape from `list_internal` for consistency.
        let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
            "SELECT
                s.id, s.submitter_id, u.claimed_handle AS preferred_username,
                s.pattern, s.proposed_label, s.description,
                s.sample_line, s.log_source, s.status, s.rejection_reason,
                s.created_at, s.updated_at,
                (SELECT COUNT(*) FROM submission_votes v WHERE v.submission_id = s.id)::BIGINT AS vote_count,
                (SELECT COUNT(*) FROM submission_flags f WHERE f.submission_id = s.id)::BIGINT AS flag_count
             FROM submissions s
             JOIN users u ON u.id = s.submitter_id
             WHERE s.status = ANY(",
        );
        let owned: Vec<String> = statuses.iter().map(|s| (*s).to_string()).collect();
        qb.push_bind(owned);
        qb.push(") ORDER BY s.updated_at DESC, s.id DESC LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        let rows: Vec<(
            Uuid,
            Uuid,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            DateTime<Utc>,
            DateTime<Utc>,
            i64,
            i64,
        )> = qb.build_query_as().fetch_all(&self.pool).await?;

        Ok(rows
            .into_iter()
            .map(|r| Submission {
                id: r.0,
                submitter_id: r.1,
                submitter_handle: display_handle(r.1, r.2),
                pattern: r.3,
                proposed_label: r.4,
                description: r.5,
                sample_line: r.6,
                log_source: r.7,
                status: SubmissionStatus::parse(&r.8).unwrap_or(SubmissionStatus::Review),
                rejection_reason: r.9,
                created_at: r.10,
                updated_at: r.11,
                vote_count: r.12,
                flag_count: r.13,
            })
            .collect())
    }

    async fn create_promoted(
        &self,
        p: PromotedSubmission<'_>,
    ) -> Result<Submission, SubmissionError> {
        let id = Uuid::now_v7();
        let inserted: Option<(Uuid,)> = sqlx::query_as(
            "INSERT INTO submissions
                (id, submitter_id, pattern, proposed_label, description, sample_line,
                 log_source, status, source_shape_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'review', $8)
             ON CONFLICT (source_shape_hash) WHERE source_shape_hash IS NOT NULL
             DO NOTHING
             RETURNING id",
        )
        .bind(id)
        .bind(p.submitter_id)
        .bind(p.pattern)
        .bind(p.proposed_label)
        .bind(p.description)
        .bind(p.sample_line)
        .bind(p.log_source)
        .bind(p.source_shape_hash)
        .fetch_optional(&self.pool)
        .await?;

        // On conflict, no row is returned -- look up the row that
        // already owns this shape hash so promotion stays idempotent
        // (same shape never creates a second submission).
        let row_id = match inserted {
            Some((row_id,)) => row_id,
            None => {
                let existing: (Uuid,) =
                    sqlx::query_as("SELECT id FROM submissions WHERE source_shape_hash = $1")
                        .bind(p.source_shape_hash)
                        .fetch_one(&self.pool)
                        .await?;
                existing.0
            }
        };

        // Re-read through the existing join-based lookup so the
        // returned row carries the same shape as `create` / `list`
        // (submitter_handle resolved via the users join, materialised
        // vote/flag counts).
        self.find_by_id(row_id, None)
            .await?
            .map(|s| s.submission)
            .ok_or(SubmissionError::NotFound)
    }

    async fn find_by_source_shape(
        &self,
        shape: &str,
    ) -> Result<Option<Submission>, SubmissionError> {
        let row: Option<(
            Uuid,
            Uuid,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            DateTime<Utc>,
            DateTime<Utc>,
            i64,
            i64,
        )> = sqlx::query_as(
            "SELECT
                s.id, s.submitter_id, u.claimed_handle AS preferred_username,
                s.pattern, s.proposed_label, s.description,
                s.sample_line, s.log_source, s.status, s.rejection_reason,
                s.created_at, s.updated_at,
                (SELECT COUNT(*) FROM submission_votes v WHERE v.submission_id = s.id)::BIGINT AS vote_count,
                (SELECT COUNT(*) FROM submission_flags f WHERE f.submission_id = s.id)::BIGINT AS flag_count
             FROM submissions s
             JOIN users u ON u.id = s.submitter_id
             WHERE s.source_shape_hash = $1",
        )
        .bind(shape)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| Submission {
            id: r.0,
            submitter_id: r.1,
            submitter_handle: display_handle(r.1, r.2),
            pattern: r.3,
            proposed_label: r.4,
            description: r.5,
            sample_line: r.6,
            log_source: r.7,
            status: SubmissionStatus::parse(&r.8).unwrap_or(SubmissionStatus::Review),
            rejection_reason: r.9,
            created_at: r.10,
            updated_at: r.11,
            vote_count: r.12,
            flag_count: r.13,
        }))
    }

    async fn mark_shipped_by_source(&self, shape: &str) -> Result<(), SubmissionError> {
        // The pattern shipped: when the linked parser rule lands, the
        // community row advances to `shipped` regardless of its prior
        // community-vote outcome (owner decision 2026-07-21).
        sqlx::query(
            "UPDATE submissions SET status = 'shipped', updated_at = NOW()
             WHERE source_shape_hash = $1",
        )
        .bind(shape)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Shared moderator-transition helper. `legal_priors` is the full set
/// of states from which we accept this transition target -- including
/// the target itself, so idempotent no-ops fall out naturally. Anything
/// outside that set returns `IllegalTransition`.
async fn admin_transition(
    pool: &PgPool,
    submission_id: Uuid,
    target: SubmissionStatus,
    reason: Option<&str>,
    legal_priors: &[SubmissionStatus],
) -> Result<SubmissionTransition, SubmissionError> {
    let mut tx = pool.begin().await?;

    // Lock the row so concurrent moderators can't both flip it. We
    // need the prior status anyway for the response payload + audit.
    let row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM submissions WHERE id = $1 FOR UPDATE")
            .bind(submission_id)
            .fetch_optional(&mut *tx)
            .await?;
    let prior_str = row.ok_or(SubmissionError::NotFound)?.0;
    let prior = SubmissionStatus::parse(&prior_str).ok_or(SubmissionError::NotFound)?;

    if !legal_priors.contains(&prior) {
        return Err(SubmissionError::IllegalTransition {
            from: prior.as_str().to_string(),
            to: target.as_str().to_string(),
        });
    }

    if prior == target {
        // Idempotent no-op: don't UPDATE, don't bump updated_at, don't
        // overwrite stored rejection_reason.
        tx.commit().await?;
        return Ok(SubmissionTransition {
            id: submission_id,
            previous_status: prior.as_str().to_string(),
            new_status: target.as_str().to_string(),
            was_changed: false,
        });
    }

    // Real transition. For reject we also persist the reason; for
    // accept/dismiss the column stays as-is (we deliberately don't
    // wipe a prior reason in case a row cycled through rejected once).
    if reason.is_some() {
        sqlx::query(
            "UPDATE submissions
             SET status = $2, rejection_reason = $3, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(submission_id)
        .bind(target.as_str())
        .bind(reason)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE submissions
             SET status = $2, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(submission_id)
        .bind(target.as_str())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(SubmissionTransition {
        id: submission_id,
        previous_status: prior.as_str().to_string(),
        new_status: target.as_str().to_string(),
        was_changed: true,
    })
}

#[derive(Debug, Clone, Copy)]
enum FindSpec {
    ById(Uuid),
    Filter(SubmissionFilter),
}

async fn list_internal(
    pool: &PgPool,
    spec: FindSpec,
    viewer_id: Option<Uuid>,
    limit: i64,
    offset: i64,
    sort: SubmissionSort,
) -> Result<Vec<SubmissionWithViewer>, SubmissionError> {
    // Parameter slot indexes are bound below; we keep this query
    // human-legible by composing the WHERE incrementally with QueryBuilder.
    let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
        "SELECT
            s.id, s.submitter_id, u.claimed_handle AS preferred_username,
            s.pattern, s.proposed_label, s.description,
            s.sample_line, s.log_source, s.status, s.rejection_reason,
            s.created_at, s.updated_at,
            (SELECT COUNT(*) FROM submission_votes v WHERE v.submission_id = s.id)::BIGINT AS vote_count,
            (SELECT COUNT(*) FROM submission_flags f WHERE f.submission_id = s.id)::BIGINT AS flag_count,
            CASE WHEN ",
    );
    qb.push_bind(viewer_id);
    qb.push(
        " IS NOT NULL AND EXISTS (
                  SELECT 1 FROM submission_votes v
                   WHERE v.submission_id = s.id AND v.user_id = ",
    );
    qb.push_bind(viewer_id);
    qb.push(") THEN TRUE ELSE FALSE END AS viewer_voted, CASE WHEN ");
    qb.push_bind(viewer_id);
    qb.push(
        " IS NOT NULL AND EXISTS (
                  SELECT 1 FROM submission_flags f
                   WHERE f.submission_id = s.id AND f.user_id = ",
    );
    qb.push_bind(viewer_id);
    qb.push(
        ") THEN TRUE ELSE FALSE END AS viewer_flagged
         FROM submissions s
         JOIN users u ON u.id = s.submitter_id
         WHERE 1=1",
    );

    match spec {
        FindSpec::ById(id) => {
            qb.push(" AND s.id = ");
            qb.push_bind(id);
        }
        FindSpec::Filter(f) => {
            if let Some(status) = f.status {
                qb.push(" AND s.status = ");
                qb.push_bind(status.as_str().to_string());
            }
            if f.mine_only {
                qb.push(" AND s.submitter_id = ");
                qb.push_bind(viewer_id);
            }
        }
    }

    qb.push(sort.order_by());
    qb.push(" LIMIT ");
    qb.push_bind(limit);
    qb.push(" OFFSET ");
    qb.push_bind(offset);

    let rows: Vec<(
        Uuid,
        Uuid,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        DateTime<Utc>,
        DateTime<Utc>,
        i64,
        i64,
        bool,
        bool,
    )> = qb.build_query_as().fetch_all(pool).await?;

    Ok(rows
        .into_iter()
        .map(|r| SubmissionWithViewer {
            submission: Submission {
                id: r.0,
                submitter_id: r.1,
                submitter_handle: display_handle(r.1, r.2),
                pattern: r.3,
                proposed_label: r.4,
                description: r.5,
                sample_line: r.6,
                log_source: r.7,
                status: SubmissionStatus::parse(&r.8).unwrap_or(SubmissionStatus::Review),
                rejection_reason: r.9,
                created_at: r.10,
                updated_at: r.11,
                vote_count: r.12,
                flag_count: r.13,
            },
            viewer_voted: r.14,
            viewer_flagged: r.15,
        })
        .collect())
}

async fn lookup_handle(pool: &PgPool, user_id: Uuid) -> Result<String, SubmissionError> {
    let h: String = sqlx::query_scalar("SELECT preferred_username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(h)
}

// -- In-memory test impl ---------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[derive(Default)]
    struct State {
        submissions: Vec<Submission>,
        votes: HashSet<(Uuid, Uuid)>,
        flags: Vec<(Uuid, Uuid, Option<String>)>,
        // Local user_id -> handle map so we can populate
        // `submitter_handle` without a real users table.
        handles: Vec<(Uuid, String)>,
        // submission id -> source_shape_hash, for promoted rows.
        // Mirrors the Postgres unique partial index: at most one entry
        // per shape hash.
        source_shapes: Vec<(Uuid, String)>,
    }

    pub struct MemorySubmissionStore {
        state: Mutex<State>,
    }

    impl Default for MemorySubmissionStore {
        fn default() -> Self {
            Self {
                state: Mutex::new(State::default()),
            }
        }
    }

    impl MemorySubmissionStore {
        pub fn add_user(&self, id: Uuid, handle: &str) {
            self.state
                .lock()
                .expect("submissions memstore poisoned")
                .handles
                .push((id, handle.to_string()));
        }

        fn handle_for(&self, id: Uuid) -> String {
            let s = self.state.lock().expect("submissions memstore poisoned");
            s.handles
                .iter()
                .find(|(uid, _)| *uid == id)
                .map(|(_, h)| h.clone())
                .unwrap_or_else(|| format!("user-{id}"))
        }
    }

    fn project(
        s: &State,
        submission: &Submission,
        viewer_id: Option<Uuid>,
    ) -> SubmissionWithViewer {
        let vote_count = s
            .votes
            .iter()
            .filter(|(sid, _)| *sid == submission.id)
            .count() as i64;
        let flag_count = s
            .flags
            .iter()
            .filter(|(sid, _, _)| *sid == submission.id)
            .count() as i64;
        let viewer_voted = matches!(viewer_id, Some(v) if s.votes.contains(&(submission.id, v)));
        let viewer_flagged = matches!(
            viewer_id,
            Some(v) if s.flags.iter().any(|(sid, uid, _)| *sid == submission.id && *uid == v)
        );
        let mut sub = Submission {
            vote_count,
            flag_count,
            ..submission.clone()
        };
        sub.submitter_handle = display_handle(sub.submitter_id, sub.submitter_handle.clone());
        SubmissionWithViewer {
            submission: sub,
            viewer_voted,
            viewer_flagged,
        }
    }

    #[async_trait]
    impl SubmissionStore for MemorySubmissionStore {
        async fn create(&self, new: NewSubmission<'_>) -> Result<Submission, SubmissionError> {
            let now = Utc::now();
            let s = Submission {
                id: Uuid::now_v7(),
                submitter_id: new.submitter_id,
                submitter_handle: display_handle(
                    new.submitter_id,
                    self.handle_for(new.submitter_id),
                ),
                pattern: new.pattern.into(),
                proposed_label: new.proposed_label.into(),
                description: new.description.into(),
                sample_line: new.sample_line.into(),
                log_source: new.log_source.into(),
                status: SubmissionStatus::Review,
                rejection_reason: None,
                created_at: now,
                updated_at: now,
                vote_count: 0,
                flag_count: 0,
            };
            self.state
                .lock()
                .expect("submissions memstore poisoned")
                .submissions
                .push(s.clone());
            Ok(s)
        }

        async fn find_by_id(
            &self,
            id: Uuid,
            viewer_id: Option<Uuid>,
        ) -> Result<Option<SubmissionWithViewer>, SubmissionError> {
            let s = self.state.lock().expect("submissions memstore poisoned");
            Ok(s.submissions
                .iter()
                .find(|s| s.id == id)
                .map(|sub| project(&s, sub, viewer_id)))
        }

        async fn list(
            &self,
            filter: SubmissionFilter,
            viewer_id: Option<Uuid>,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<SubmissionWithViewer>, SubmissionError> {
            let state = self.state.lock().expect("submissions memstore poisoned");
            let mut rows: Vec<SubmissionWithViewer> = state
                .submissions
                .iter()
                .filter(|s| match filter.status {
                    Some(want) => s.status == want,
                    None => true,
                })
                .filter(|s| {
                    if !filter.mine_only {
                        return true;
                    }
                    matches!(viewer_id, Some(v) if s.submitter_id == v)
                })
                .map(|s| project(&state, s, viewer_id))
                .collect();
            match filter.sort {
                SubmissionSort::Newest => rows.sort_by(|a, b| {
                    b.submission
                        .updated_at
                        .cmp(&a.submission.updated_at)
                        .then_with(|| b.submission.id.cmp(&a.submission.id))
                }),
                SubmissionSort::Oldest => rows.sort_by(|a, b| {
                    a.submission
                        .updated_at
                        .cmp(&b.submission.updated_at)
                        .then_with(|| a.submission.id.cmp(&b.submission.id))
                }),
                SubmissionSort::MostVotes => rows.sort_by(|a, b| {
                    b.submission
                        .vote_count
                        .cmp(&a.submission.vote_count)
                        .then_with(|| b.submission.updated_at.cmp(&a.submission.updated_at))
                        .then_with(|| b.submission.id.cmp(&a.submission.id))
                }),
            }
            let start = offset.max(0) as usize;
            let take = limit.max(0) as usize;
            Ok(rows.into_iter().skip(start).take(take).collect())
        }

        async fn vote(
            &self,
            submission_id: Uuid,
            user_id: Uuid,
        ) -> Result<WriteOutcome, SubmissionError> {
            let mut s = self.state.lock().expect("submissions memstore poisoned");
            if s.votes.insert((submission_id, user_id)) {
                if let Some(sub) = s.submissions.iter_mut().find(|x| x.id == submission_id) {
                    sub.updated_at = Utc::now();
                }
                Ok(WriteOutcome::Inserted)
            } else {
                Ok(WriteOutcome::AlreadyExists)
            }
        }

        async fn unvote(
            &self,
            submission_id: Uuid,
            user_id: Uuid,
        ) -> Result<WriteOutcome, SubmissionError> {
            let mut s = self.state.lock().expect("submissions memstore poisoned");
            if s.votes.remove(&(submission_id, user_id)) {
                Ok(WriteOutcome::Inserted)
            } else {
                Ok(WriteOutcome::AlreadyExists)
            }
        }

        async fn flag(
            &self,
            submission_id: Uuid,
            user_id: Uuid,
            reason: Option<&str>,
        ) -> Result<FlagOutcome, SubmissionError> {
            let mut state = self.state.lock().expect("submissions memstore poisoned");
            let prior = state
                .submissions
                .iter()
                .find(|s| s.id == submission_id)
                .map(|s| s.status)
                .ok_or(SubmissionError::NotFound)?;
            let already = state
                .flags
                .iter()
                .any(|(sid, uid, _)| *sid == submission_id && *uid == user_id);
            let write = if already {
                WriteOutcome::AlreadyExists
            } else {
                state
                    .flags
                    .push((submission_id, user_id, reason.map(str::to_string)));
                WriteOutcome::Inserted
            };
            let count = state
                .flags
                .iter()
                .filter(|(sid, _, _)| *sid == submission_id)
                .count() as i64;
            let escalated = matches!(write, WriteOutcome::Inserted)
                && prior == SubmissionStatus::Review
                && count >= AUTO_FLAG_THRESHOLD;
            if escalated {
                if let Some(sub) = state.submissions.iter_mut().find(|s| s.id == submission_id) {
                    sub.status = SubmissionStatus::Flagged;
                    sub.updated_at = Utc::now();
                }
            }
            Ok(FlagOutcome {
                write,
                flag_count: count,
                escalated,
            })
        }

        async fn withdraw(
            &self,
            submission_id: Uuid,
            caller_id: Uuid,
        ) -> Result<Submission, SubmissionError> {
            let mut state = self.state.lock().expect("submissions memstore poisoned");
            let sub = state
                .submissions
                .iter_mut()
                .find(|s| s.id == submission_id)
                .ok_or(SubmissionError::NotFound)?;
            if sub.submitter_id != caller_id {
                return Err(SubmissionError::Forbidden);
            }
            if sub.status != SubmissionStatus::Review {
                return Err(SubmissionError::BadState);
            }
            sub.status = SubmissionStatus::Withdrawn;
            sub.updated_at = Utc::now();
            Ok(sub.clone())
        }

        async fn accept_submission(
            &self,
            submission_id: Uuid,
            _moderator_id: Uuid,
        ) -> Result<SubmissionTransition, SubmissionError> {
            mem_transition(
                self,
                submission_id,
                SubmissionStatus::Accepted,
                None,
                &[
                    SubmissionStatus::Review,
                    SubmissionStatus::Flagged,
                    SubmissionStatus::Accepted,
                ],
            )
        }

        async fn reject_submission(
            &self,
            submission_id: Uuid,
            _moderator_id: Uuid,
            reason: &str,
        ) -> Result<SubmissionTransition, SubmissionError> {
            mem_transition(
                self,
                submission_id,
                SubmissionStatus::Rejected,
                Some(reason),
                &[
                    SubmissionStatus::Review,
                    SubmissionStatus::Flagged,
                    SubmissionStatus::Rejected,
                ],
            )
        }

        async fn dismiss_flag(
            &self,
            submission_id: Uuid,
            _moderator_id: Uuid,
        ) -> Result<SubmissionTransition, SubmissionError> {
            mem_transition(
                self,
                submission_id,
                SubmissionStatus::Review,
                None,
                &[SubmissionStatus::Flagged, SubmissionStatus::Review],
            )
        }

        async fn list_admin_queue(
            &self,
            filter: AdminQueueFilter,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<Submission>, SubmissionError> {
            let state = self.state.lock().expect("submissions memstore poisoned");
            let want = |s: SubmissionStatus| match filter {
                AdminQueueFilter::Review => s == SubmissionStatus::Review,
                AdminQueueFilter::Flagged => s == SubmissionStatus::Flagged,
                AdminQueueFilter::All => {
                    s == SubmissionStatus::Review || s == SubmissionStatus::Flagged
                }
            };
            let mut rows: Vec<Submission> = state
                .submissions
                .iter()
                .filter(|s| want(s.status))
                .map(|s| {
                    let vote_count =
                        state.votes.iter().filter(|(sid, _)| *sid == s.id).count() as i64;
                    let flag_count = state
                        .flags
                        .iter()
                        .filter(|(sid, _, _)| *sid == s.id)
                        .count() as i64;
                    let mut out = Submission {
                        vote_count,
                        flag_count,
                        ..s.clone()
                    };
                    out.submitter_handle =
                        display_handle(out.submitter_id, out.submitter_handle.clone());
                    out
                })
                .collect();
            rows.sort_by(|a, b| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then_with(|| b.id.cmp(&a.id))
            });
            let start = offset.max(0) as usize;
            let take = limit.max(0) as usize;
            Ok(rows.into_iter().skip(start).take(take).collect())
        }

        async fn create_promoted(
            &self,
            p: PromotedSubmission<'_>,
        ) -> Result<Submission, SubmissionError> {
            let mut state = self.state.lock().expect("submissions memstore poisoned");

            // Idempotency: a submission already linked to this shape
            // hash wins over inserting a duplicate.
            let existing_id = state
                .source_shapes
                .iter()
                .find(|(_, sh)| sh == p.source_shape_hash)
                .map(|(sid, _)| *sid);
            if let Some(sid) = existing_id {
                let sub = state
                    .submissions
                    .iter()
                    .find(|s| s.id == sid)
                    .cloned()
                    .expect("source_shapes entry without matching submission");
                return Ok(project(&state, &sub, None).submission);
            }

            let now = Utc::now();
            let s = Submission {
                id: Uuid::now_v7(),
                submitter_id: p.submitter_id,
                submitter_handle: p.submitter_handle.to_string(),
                pattern: p.pattern.into(),
                proposed_label: p.proposed_label.into(),
                description: p.description.into(),
                sample_line: p.sample_line.into(),
                log_source: p.log_source.into(),
                status: SubmissionStatus::Review,
                rejection_reason: None,
                created_at: now,
                updated_at: now,
                vote_count: 0,
                flag_count: 0,
            };
            state
                .source_shapes
                .push((s.id, p.source_shape_hash.to_string()));
            state.submissions.push(s.clone());
            // Return through `project` so the community display-handle
            // override applies to the freshly-inserted row too.
            Ok(project(&state, &s, None).submission)
        }

        async fn find_by_source_shape(
            &self,
            shape: &str,
        ) -> Result<Option<Submission>, SubmissionError> {
            let state = self.state.lock().expect("submissions memstore poisoned");
            let sid = state
                .source_shapes
                .iter()
                .find(|(_, sh)| sh == shape)
                .map(|(sid, _)| *sid);
            Ok(sid
                .and_then(|sid| state.submissions.iter().find(|s| s.id == sid))
                .map(|s| project(&state, s, None).submission))
        }

        async fn mark_shipped_by_source(&self, shape: &str) -> Result<(), SubmissionError> {
            let mut state = self.state.lock().expect("submissions memstore poisoned");
            let sid = state
                .source_shapes
                .iter()
                .find(|(_, sh)| sh == shape)
                .map(|(sid, _)| *sid);
            if let Some(sid) = sid {
                if let Some(sub) = state.submissions.iter_mut().find(|s| s.id == sid) {
                    // The pattern shipped: advance regardless of prior
                    // community-vote outcome (owner decision 2026-07-21).
                    sub.status = SubmissionStatus::Shipped;
                    sub.updated_at = Utc::now();
                }
            }
            Ok(())
        }
    }

    /// In-memory mirror of the Postgres `admin_transition` helper. Same
    /// idempotent / illegal-transition semantics so route tests that
    /// drive the memory store reproduce the production behaviour.
    fn mem_transition(
        store: &MemorySubmissionStore,
        submission_id: Uuid,
        target: SubmissionStatus,
        reason: Option<&str>,
        legal_priors: &[SubmissionStatus],
    ) -> Result<SubmissionTransition, SubmissionError> {
        let mut state = store.state.lock().expect("submissions memstore poisoned");
        let sub = state
            .submissions
            .iter_mut()
            .find(|s| s.id == submission_id)
            .ok_or(SubmissionError::NotFound)?;
        let prior = sub.status;
        if !legal_priors.contains(&prior) {
            return Err(SubmissionError::IllegalTransition {
                from: prior.as_str().to_string(),
                to: target.as_str().to_string(),
            });
        }
        if prior == target {
            return Ok(SubmissionTransition {
                id: submission_id,
                previous_status: prior.as_str().to_string(),
                new_status: target.as_str().to_string(),
                was_changed: false,
            });
        }
        sub.status = target;
        sub.updated_at = Utc::now();
        if let Some(r) = reason {
            sub.rejection_reason = Some(r.to_string());
        }
        Ok(SubmissionTransition {
            id: submission_id,
            previous_status: prior.as_str().to_string(),
            new_status: target.as_str().to_string(),
            was_changed: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemorySubmissionStore;
    use super::*;

    fn fresh_store_with_user(handle: &str) -> (MemorySubmissionStore, Uuid) {
        let store = MemorySubmissionStore::default();
        let user_id = Uuid::now_v7();
        store.add_user(user_id, handle);
        (store, user_id)
    }

    fn promoted<'a>(shape: &'a str, submitter_id: Uuid, handle: &'a str) -> PromotedSubmission<'a> {
        PromotedSubmission {
            submitter_id,
            submitter_handle: handle,
            pattern: "<X> *",
            proposed_label: "x_event",
            description: "promoted from a tray parser shape",
            sample_line: "<X> sample line",
            log_source: "live",
            source_shape_hash: shape,
        }
    }

    #[tokio::test]
    async fn create_promoted_is_idempotent_on_shape() {
        let store = MemorySubmissionStore::default();
        let p = promoted("sh_a", COMMUNITY_USER_ID, "community");
        let a = store.create_promoted(p.clone()).await.unwrap();
        let b = store.create_promoted(p).await.unwrap(); // second publish
        assert_eq!(a.id, b.id, "same shape must not create a second row");
        assert_eq!(a.status, SubmissionStatus::Review);
        assert_eq!(a.submitter_id, COMMUNITY_USER_ID);
    }

    #[tokio::test]
    async fn community_owner_always_displays_as_community_handle() {
        let store = MemorySubmissionStore::default();
        // Promote an anonymous shape owned by the seeded community system
        // account, deliberately passing the *unclaimable* raw seed handle
        // (`community.system`) to prove the display override wins no matter
        // what backs the account in the users table.
        let row = store
            .create_promoted(promoted("sh_disp", COMMUNITY_USER_ID, "community.system"))
            .await
            .unwrap();
        assert_eq!(row.submitter_handle, "community");
        // Every subsequent read path funnels through the shared
        // `display_handle`, so both surface `community` too.
        let by_shape = store
            .find_by_source_shape("sh_disp")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_shape.submitter_handle, "community");
        let by_id = store.find_by_id(row.id, None).await.unwrap().unwrap();
        assert_eq!(by_id.submission.submitter_handle, "community");
    }

    #[tokio::test]
    async fn mark_shipped_by_source_advances_linked_row() {
        let store = MemorySubmissionStore::default();
        let row = store
            .create_promoted(promoted("sh_b", COMMUNITY_USER_ID, "community"))
            .await
            .unwrap();
        store.mark_shipped_by_source("sh_b").await.unwrap();
        let got = store.find_by_source_shape("sh_b").await.unwrap().unwrap();
        assert_eq!(got.id, row.id);
        assert_eq!(got.status, SubmissionStatus::Shipped);
    }

    #[tokio::test]
    async fn mark_shipped_by_source_ships_regardless_of_prior_outcome() {
        let store = MemorySubmissionStore::default();
        let row = store
            .create_promoted(promoted("sh_terminal", COMMUNITY_USER_ID, "community"))
            .await
            .unwrap();
        // Reject the promoted row before the parser rule ships -- a
        // moderator judged this shape unworthy of publication.
        let moderator_id = Uuid::now_v7();
        store
            .reject_submission(row.id, moderator_id, "not useful")
            .await
            .unwrap();
        store.mark_shipped_by_source("sh_terminal").await.unwrap();
        let got = store
            .find_by_source_shape("sh_terminal")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.id, row.id);
        assert_eq!(
            got.status,
            SubmissionStatus::Shipped,
            "when the parser rule lands, the community row ships regardless of prior outcome"
        );
    }

    #[tokio::test]
    async fn create_then_find_returns_zero_counts() {
        let (store, alice) = fresh_store_with_user("alice");
        let s = store
            .create(NewSubmission {
                submitter_id: alice,
                pattern: "<X> *",
                proposed_label: "x_event",
                description: "test",
                sample_line: "<X> y",
                log_source: "live",
            })
            .await
            .expect("create");
        let fetched = store
            .find_by_id(s.id, Some(alice))
            .await
            .expect("find")
            .expect("present");
        assert_eq!(fetched.submission.vote_count, 0);
        assert_eq!(fetched.submission.flag_count, 0);
        assert!(!fetched.viewer_voted);
        assert!(!fetched.viewer_flagged);
        assert_eq!(fetched.submission.status, SubmissionStatus::Review);
    }

    #[tokio::test]
    async fn list_honours_sort_order() {
        let (store, alice) = fresh_store_with_user("alice");
        let bob = Uuid::now_v7();
        store.add_user(bob, "bob");
        let carol = Uuid::now_v7();
        store.add_user(carol, "carol");

        let mk = |p: &'static str| NewSubmission {
            submitter_id: alice,
            pattern: p,
            proposed_label: p,
            description: "d",
            sample_line: "s",
            log_source: "live",
        };
        let s1 = store.create(mk("a")).await.unwrap();
        let s2 = store.create(mk("b")).await.unwrap();
        let s3 = store.create(mk("c")).await.unwrap();

        // Bump updated_at in a known order (s1 oldest, then s3, then s2
        // newest — each vote restamps updated_at) and give s2 the most
        // votes (3 vs 1).
        store.vote(s1.id, alice).await.unwrap();
        store.vote(s3.id, alice).await.unwrap();
        store.vote(s2.id, alice).await.unwrap();
        store.vote(s2.id, bob).await.unwrap();
        store.vote(s2.id, carol).await.unwrap();

        let ids = |rows: &[SubmissionWithViewer]| {
            rows.iter().map(|r| r.submission.id).collect::<Vec<_>>()
        };
        let list_with = |sort| {
            let store = &store;
            async move {
                store
                    .list(
                        SubmissionFilter {
                            sort,
                            ..Default::default()
                        },
                        Some(alice),
                        50,
                        0,
                    )
                    .await
                    .unwrap()
            }
        };

        let newest = list_with(SubmissionSort::Newest).await;
        let oldest = list_with(SubmissionSort::Oldest).await;
        let votes = list_with(SubmissionSort::MostVotes).await;

        let newest_ids = ids(&newest);
        let mut oldest_rev = ids(&oldest);
        oldest_rev.reverse();
        assert_eq!(newest_ids, oldest_rev, "oldest is the reverse of newest");
        assert_eq!(newest_ids[0], s2.id, "s2 was updated most recently");
        assert_eq!(votes[0].submission.id, s2.id, "s2 has the most votes");
    }

    #[tokio::test]
    async fn vote_is_idempotent_per_user() {
        let (store, alice) = fresh_store_with_user("alice");
        let bob = Uuid::now_v7();
        store.add_user(bob, "bob");
        let s = store
            .create(NewSubmission {
                submitter_id: alice,
                pattern: "p",
                proposed_label: "p",
                description: "p",
                sample_line: "p",
                log_source: "live",
            })
            .await
            .unwrap();

        assert_eq!(store.vote(s.id, bob).await.unwrap(), WriteOutcome::Inserted);
        assert_eq!(
            store.vote(s.id, bob).await.unwrap(),
            WriteOutcome::AlreadyExists
        );
        let row = store.find_by_id(s.id, Some(bob)).await.unwrap().unwrap();
        assert_eq!(row.submission.vote_count, 1);
        assert!(row.viewer_voted);
    }

    #[tokio::test]
    async fn flag_auto_escalates_at_threshold() {
        let (store, alice) = fresh_store_with_user("alice");
        let s = store
            .create(NewSubmission {
                submitter_id: alice,
                pattern: "p",
                proposed_label: "p",
                description: "p",
                sample_line: "p",
                log_source: "live",
            })
            .await
            .unwrap();

        let flaggers: Vec<Uuid> = (0..AUTO_FLAG_THRESHOLD).map(|_| Uuid::now_v7()).collect();
        for (i, uid) in flaggers.iter().enumerate() {
            let outcome = store.flag(s.id, *uid, Some("nope")).await.unwrap();
            assert_eq!(outcome.flag_count, i as i64 + 1);
            // Only the *threshold-th* flag should fire escalation;
            // earlier ones shouldn't, later (above-threshold) ones
            // shouldn't either.
            if i as i64 + 1 == AUTO_FLAG_THRESHOLD {
                assert!(outcome.escalated, "threshold flag should escalate");
            } else {
                assert!(!outcome.escalated, "non-threshold flag should not escalate");
            }
        }

        // One more flag from a fresh user — count goes up but
        // status is already flagged, so no second escalation.
        let extra = Uuid::now_v7();
        let outcome = store.flag(s.id, extra, None).await.unwrap();
        assert!(!outcome.escalated);
        assert_eq!(outcome.flag_count, AUTO_FLAG_THRESHOLD + 1);

        let fetched = store.find_by_id(s.id, None).await.unwrap().unwrap();
        assert_eq!(fetched.submission.status, SubmissionStatus::Flagged);
    }

    #[tokio::test]
    async fn flag_is_idempotent_per_user() {
        let (store, alice) = fresh_store_with_user("alice");
        let bob = Uuid::now_v7();
        let s = store
            .create(NewSubmission {
                submitter_id: alice,
                pattern: "p",
                proposed_label: "p",
                description: "p",
                sample_line: "p",
                log_source: "live",
            })
            .await
            .unwrap();
        assert_eq!(
            store.flag(s.id, bob, None).await.unwrap().write,
            WriteOutcome::Inserted
        );
        let dup = store.flag(s.id, bob, Some("again")).await.unwrap();
        assert_eq!(dup.write, WriteOutcome::AlreadyExists);
        // Still only 1 distinct flagger -> not escalated.
        assert_eq!(dup.flag_count, 1);
        assert!(!dup.escalated);
    }

    #[tokio::test]
    async fn withdraw_only_by_submitter_and_only_in_review() {
        let (store, alice) = fresh_store_with_user("alice");
        let bob = Uuid::now_v7();
        let s = store
            .create(NewSubmission {
                submitter_id: alice,
                pattern: "p",
                proposed_label: "p",
                description: "p",
                sample_line: "p",
                log_source: "live",
            })
            .await
            .unwrap();

        // Other user can't withdraw.
        let err = store.withdraw(s.id, bob).await.unwrap_err();
        assert!(matches!(err, SubmissionError::Forbidden));

        // Submitter can.
        let withdrawn = store.withdraw(s.id, alice).await.unwrap();
        assert_eq!(withdrawn.status, SubmissionStatus::Withdrawn);

        // Second attempt fails because state is no longer review.
        let err = store.withdraw(s.id, alice).await.unwrap_err();
        assert!(matches!(err, SubmissionError::BadState));
    }

    #[tokio::test]
    async fn list_filters_by_status_and_mine_only() {
        let (store, alice) = fresh_store_with_user("alice");
        let bob = Uuid::now_v7();
        store.add_user(bob, "bob");

        let _alice1 = store
            .create(NewSubmission {
                submitter_id: alice,
                pattern: "a1",
                proposed_label: "a1",
                description: "a",
                sample_line: "a",
                log_source: "live",
            })
            .await
            .unwrap();
        let bob1 = store
            .create(NewSubmission {
                submitter_id: bob,
                pattern: "b1",
                proposed_label: "b1",
                description: "b",
                sample_line: "b",
                log_source: "live",
            })
            .await
            .unwrap();
        store.withdraw(bob1.id, bob).await.unwrap();

        // No filter, viewer = alice -> sees 2 submissions, viewer_voted false.
        let all = store
            .list(SubmissionFilter::default(), Some(alice), 50, 0)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);

        // status=review -> alice's only.
        let review = store
            .list(
                SubmissionFilter {
                    status: Some(SubmissionStatus::Review),
                    mine_only: false,
                    ..Default::default()
                },
                Some(alice),
                50,
                0,
            )
            .await
            .unwrap();
        assert_eq!(review.len(), 1);
        assert_eq!(review[0].submission.submitter_id, alice);

        // mine_only with viewer=bob -> bob's only (the withdrawn one).
        let mine = store
            .list(
                SubmissionFilter {
                    status: None,
                    mine_only: true,
                    ..Default::default()
                },
                Some(bob),
                50,
                0,
            )
            .await
            .unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].submission.submitter_id, bob);
    }
}
