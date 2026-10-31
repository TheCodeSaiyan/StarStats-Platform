//! RSI handle renames.
//!
//! A signed-in user may change `users.claimed_handle`, for a typo or
//! because their RSI handle really changed. The rename:
//!
//!  - rewrites the handle in place in every handle-keyed table, in one
//!    Postgres transaction, so every existing query keeps working;
//!  - resets RSI verification, because the new handle is unproven;
//!  - holds the old handle for [`RESERVATION`] so nobody else can claim
//!    a just-freed name, and so links built on it can redirect;
//!  - starts a [`COOLDOWN`] before the next rename. A never-verified
//!    account's first rename skips it once, because that is the typo fix.
//!
//! Backed by table `handle_renames` and `users.last_handle_change_at`
//! (migration 0081). The rules live in [`evaluate`], a pure function, so
//! they are tested without a store; each store re-evaluates them under a
//! row lock so two concurrent renames cannot both pass the cooldown.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// Minimum time between two renames of the same account.
pub const COOLDOWN: Duration = Duration::days(30);

/// How long an old handle stays held for its previous owner. Matches
/// the device-token lifetime, so every token minted under the old
/// handle has expired by the time the name is released.
pub const RESERVATION: Duration = Duration::days(90);

/// What the rules need to know about the account being renamed.
#[derive(Debug, Clone)]
pub struct RenameSubject {
    pub current_handle: String,
    pub rsi_verified: bool,
    pub last_handle_change_at: Option<DateTime<Utc>>,
}

/// Which kind of rename the rules allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameKind {
    /// The one cooldown-free change of a never-verified, never-renamed
    /// account.
    Free,
    /// Any other rename; the cooldown had passed.
    Standard,
}

/// Why a rename was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenameRefusal {
    #[error("handle is not valid")]
    InvalidHandle,
    #[error("new handle is the same as the current one")]
    Unchanged,
    #[error("a rename is not allowed until {until}")]
    Cooldown { until: DateTime<Utc> },
    #[error("handle is already claimed")]
    Taken,
    #[error("handle is reserved after another account's rename")]
    Reserved,
}

/// Apply the rename rules. Availability of the new handle (taken or
/// reserved by someone else) is the store's to check, because it needs
/// the database; everything else is decided here.
pub fn evaluate(
    subject: &RenameSubject,
    new_handle: &str,
    now: DateTime<Utc>,
) -> Result<RenameKind, RenameRefusal> {
    if !crate::validation::validate_handle(new_handle) {
        return Err(RenameRefusal::InvalidHandle);
    }
    // Byte-equal is a no-op. A change of case only is a real rename:
    // it fixes how the name is shown, and RSI handles are case-preserving.
    if subject.current_handle == new_handle {
        return Err(RenameRefusal::Unchanged);
    }
    match subject.last_handle_change_at {
        None if !subject.rsi_verified => Ok(RenameKind::Free),
        None => Ok(RenameKind::Standard),
        Some(last) => {
            let until = last + COOLDOWN;
            if now < until {
                Err(RenameRefusal::Cooldown { until })
            } else {
                Ok(RenameKind::Standard)
            }
        }
    }
}

/// One completed rename, as recorded in `handle_renames`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandleRename {
    pub id: Uuid,
    pub user_id: Uuid,
    pub old_handle: String,
    pub new_handle: String,
    pub was_verified: bool,
    pub free_change: bool,
    pub renamed_at: DateTime<Utc>,
    pub reserved_until: DateTime<Utc>,
    pub spicedb_moved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum HandleRenameError {
    #[error(transparent)]
    Refused(#[from] RenameRefusal),
    #[error("user not found")]
    UserNotFound,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait HandleRenameStore: Send + Sync + 'static {
    /// Rename the user's handle everywhere, reset RSI verification and
    /// record the rename. Applies [`evaluate`] under a lock on the user's
    /// row, then refuses a new handle that another account holds or has
    /// reserved.
    async fn rename(
        &self,
        user_id: Uuid,
        new_handle: &str,
        now: DateTime<Utc>,
    ) -> Result<HandleRename, HandleRenameError>;

    /// Where a link built on `old_handle` should go: the current handle
    /// of the user who renamed away from it, while the reservation lasts.
    /// `None` when the handle is held by a live account or nothing applies.
    async fn redirect_for(
        &self,
        old_handle: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, HandleRenameError>;

    /// The user's most recent rename, for the settings pane.
    async fn latest_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Option<HandleRename>, HandleRenameError>;

    /// Renames whose SpiceDB move has not landed, oldest first.
    async fn pending_spicedb_moves(
        &self,
        limit: i64,
    ) -> Result<Vec<HandleRename>, HandleRenameError>;

    async fn mark_spicedb_moved(
        &self,
        id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), HandleRenameError>;

    /// Every rename at or after `since`, oldest first. Feeds [`HandleIndex`].
    async fn recent(&self, since: DateTime<Utc>) -> Result<Vec<HandleRename>, HandleRenameError>;
}

/// What the sign-in check needs to know about one renamed account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedAccount {
    pub current_handle: String,
    /// Every handle the account renamed away from while it is still held,
    /// newest first. Ingest accepts a batch stamped with one of these.
    pub previous_handles: Vec<String>,
    pub last_renamed_at: DateTime<Utc>,
}

/// In-process index of accounts renamed within the reservation window,
/// keyed by user id (the JWT `sub`).
///
/// Tokens carry the handle in `preferred_username`: web tokens for an
/// hour, device tokens for 90 days, and most handlers trust that copy.
/// The auth extractor asks this index whether a token predates a rename
/// and, if so, swaps in the current handle, so an old token can neither
/// read the wrong data nor write rows under a handle that has gone.
///
/// The rename route updates it at once; [`spawn_handle_index_refresh`]
/// reloads it from the database every minute, which is how a rename made
/// on another server instance arrives. Renames are rare, so the map is
/// small and the per-request cost is one read lock.
#[derive(Default)]
pub struct HandleIndex {
    accounts: std::sync::RwLock<std::collections::HashMap<Uuid, RenamedAccount>>,
}

impl HandleIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, user_id: Uuid) -> Option<RenamedAccount> {
        self.accounts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&user_id)
            .cloned()
    }

    /// Rebuild from renames, oldest first, as [`HandleRenameStore::recent`]
    /// returns them.
    pub fn replace_all(&self, renames: &[HandleRename]) {
        let mut map: std::collections::HashMap<Uuid, RenamedAccount> = Default::default();
        for r in renames {
            Self::apply(&mut map, r);
        }
        *self.accounts.write().unwrap_or_else(|e| e.into_inner()) = map;
    }

    /// Add one rename that has just committed.
    pub fn record(&self, r: &HandleRename) {
        Self::apply(
            &mut self.accounts.write().unwrap_or_else(|e| e.into_inner()),
            r,
        );
    }

    fn apply(map: &mut std::collections::HashMap<Uuid, RenamedAccount>, r: &HandleRename) {
        let entry = map.entry(r.user_id).or_insert_with(|| RenamedAccount {
            current_handle: r.new_handle.clone(),
            previous_handles: Vec::new(),
            last_renamed_at: r.renamed_at,
        });
        entry.current_handle = r.new_handle.clone();
        entry.last_renamed_at = r.renamed_at;
        entry.previous_handles.retain(|h| {
            !h.eq_ignore_ascii_case(&r.old_handle) && !h.eq_ignore_ascii_case(&r.new_handle)
        });
        entry.previous_handles.insert(0, r.old_handle.clone());
    }

    /// The handle a token should act as. `Some(current)` when the token
    /// was issued no later than the account's last rename and names a
    /// different handle; `None` when the token is already right.
    pub fn resolve(&self, user_id: Uuid, token_handle: &str, issued_at: usize) -> Option<String> {
        let acct = self.get(user_id)?;
        if acct.current_handle.eq_ignore_ascii_case(token_handle) {
            return None;
        }
        // JWT `iat` has one-second resolution; a token minted in the same
        // second as the rename but before it must still be swapped.
        let renamed = usize::try_from(acct.last_renamed_at.timestamp()).unwrap_or(0);
        (issued_at <= renamed).then_some(acct.current_handle)
    }
}

/// How often [`HandleIndex`] reloads from the database.
pub const INDEX_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

pub fn spawn_handle_index_refresh(
    store: std::sync::Arc<dyn HandleRenameStore>,
    index: std::sync::Arc<HandleIndex>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match store.recent(Utc::now() - RESERVATION).await {
                Ok(renames) => index.replace_all(&renames),
                Err(e) => tracing::warn!(error = %e, "handle index refresh failed"),
            }
            tokio::time::sleep(INDEX_REFRESH_INTERVAL).await;
        }
    })
}

/// Moves a handle's authorization relationships. `SpicedbClient` in
/// production; a fake in tests.
#[async_trait]
pub trait HandleMover: Send + Sync {
    async fn move_handle(&self, old: &str, new: &str) -> anyhow::Result<usize>;
}

#[async_trait]
impl HandleMover for crate::spicedb::SpicedbClient {
    async fn move_handle(&self, old: &str, new: &str) -> anyhow::Result<usize> {
        crate::spicedb::SpicedbClient::move_handle(self, old, new).await
    }
}

/// How often the reconciler looks for renames whose SpiceDB move did not
/// land, and how many it takes per pass.
pub const RECONCILE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);
const RECONCILE_BATCH: i64 = 50;

/// One reconciler pass: retry the SpiceDB move of every rename that has
/// none recorded, oldest first. Returns how many landed. A failure is
/// logged and left for the next pass; the move is idempotent, so a retry
/// after a partial move is safe.
pub async fn reconcile_spicedb_moves(
    store: &dyn HandleRenameStore,
    mover: &dyn HandleMover,
) -> Result<usize, HandleRenameError> {
    let mut landed = 0;
    for r in store.pending_spicedb_moves(RECONCILE_BATCH).await? {
        match mover.move_handle(&r.old_handle, &r.new_handle).await {
            Ok(_) => {
                store.mark_spicedb_moved(r.id, Utc::now()).await?;
                landed += 1;
            }
            Err(e) => {
                tracing::warn!(error = %e, rename_id = %r.id, "SpiceDB handle move retry failed");
            }
        }
    }
    Ok(landed)
}

/// Run [`reconcile_spicedb_moves`] every [`RECONCILE_INTERVAL`] for the
/// life of the process.
pub fn spawn_spicedb_reconciler(
    store: std::sync::Arc<dyn HandleRenameStore>,
    mover: std::sync::Arc<dyn HandleMover>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(RECONCILE_INTERVAL).await;
            match reconcile_spicedb_moves(store.as_ref(), mover.as_ref()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(landed = n, "SpiceDB handle moves reconciled"),
                Err(e) => tracing::warn!(error = %e, "SpiceDB handle-move reconciler pass failed"),
            }
        }
    })
}

/// Every column outside `users` that holds a player's handle, as
/// `(table, column)`. The rename rewrites each one in place.
///
/// Deliberately absent:
///  - `audit_log.actor_handle` and handles inside `audit_log.payload`.
///    The log is append-only (a trigger rejects UPDATE) and hash-chained;
///    it records who someone was at the time, which is what an audit
///    trail is for.
///  - `events.payload` (`player`, `killer`, `victim`). Stats queries match
///    those against the player's previous handles from this table rather
///    than rewriting every stored event.
///  - `chat_rooms.dm_a` / `dm_b` and `chat_room_members.handle`. These are
///    chat (Matrix) localparts, and the player keeps their chat account
///    across a rename (`users.chat_localpart`, migration 0082).
///
/// Adding a handle-keyed column anywhere means adding it here; the
/// Postgres integration test checks every entry and fails on a miss.
pub const HANDLE_COLUMNS: &[(&str, &str)] = &[
    ("events", "claimed_handle"),
    ("quarantined_events", "claimed_handle"),
    ("stat_event_counts", "claimed_handle"),
    ("entity_rollup_agg", "claimed_handle"),
    ("session_summary", "claimed_handle"),
    ("character_records", "claimed_handle"),
    ("stat_rollup_state", "claimed_handle"),
    ("contract_runs", "claimed_handle"),
    ("unknown_tag_sighting", "claimed_handle"),
    ("share_metadata", "owner_handle"),
    ("share_metadata", "recipient_handle"),
    ("share_reports", "reporter_handle"),
    ("share_reports", "owner_handle"),
    ("share_reports", "recipient_handle"),
    ("share_reports", "resolved_by"),
    ("public_profile_view_counters", "profile_handle"),
    ("friend_requests", "requester_handle"),
    ("friend_requests", "recipient_handle"),
    ("friendships", "handle_a"),
    ("friendships", "handle_b"),
    ("user_blocks", "blocker_handle"),
    ("user_blocks", "blocked_handle"),
    ("user_mutes", "muter_handle"),
    ("user_mutes", "muted_handle"),
    ("notifications", "recipient_handle"),
    ("notifications", "actor_handle"),
    ("profile_salutes", "saluter_handle"),
    ("profile_salutes", "target_handle"),
    ("lfg_posts", "host_handle"),
    ("lfg_posts", "removed_by"),
    ("lfg_members", "member_handle"),
    ("lfg_reports", "reporter_handle"),
    ("lfg_reports", "host_handle"),
    ("lfg_reports", "resolved_by"),
    ("crew_history", "handle"),
    ("crew_history", "other_handle"),
    ("crew_commends", "giver_handle"),
    ("crew_commends", "recipient_handle"),
    ("chat_reports", "reporter_handle"),
    ("chat_reports", "reported_handle"),
    ("chat_reports", "resolved_by"),
    ("parser_submissions", "submitter_handle"),
    ("account_restrictions", "restricted_by"),
    ("news_posts", "created_by"),
    ("roadmap_changelog", "published_by"),
    ("parser_health_finding", "acknowledged_by"),
];

type RenameRow = (
    Uuid,
    Uuid,
    String,
    String,
    bool,
    bool,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

const RENAME_COLUMNS: &str = "id, user_id, old_handle, new_handle, was_verified, free_change, \
                              renamed_at, reserved_until, spicedb_moved_at";

fn rename_from_row(r: RenameRow) -> HandleRename {
    HandleRename {
        id: r.0,
        user_id: r.1,
        old_handle: r.2,
        new_handle: r.3,
        was_verified: r.4,
        free_change: r.5,
        renamed_at: r.6,
        reserved_until: r.7,
        spicedb_moved_at: r.8,
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.code().as_deref() == Some("23505"))
}

pub struct PostgresHandleRenameStore {
    pool: sqlx::PgPool,
}

impl PostgresHandleRenameStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HandleRenameStore for PostgresHandleRenameStore {
    async fn rename(
        &self,
        user_id: Uuid,
        new_handle: &str,
        now: DateTime<Utc>,
    ) -> Result<HandleRename, HandleRenameError> {
        let mut tx = self.pool.begin().await?;

        // The row lock serialises renames of one account, so two requests
        // cannot both pass the cooldown.
        let row: Option<(String, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT claimed_handle, rsi_verified_at, last_handle_change_at \
             FROM users WHERE id = $1 FOR UPDATE",
        )
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (old_handle, rsi_verified_at, last_handle_change_at) =
            row.ok_or(HandleRenameError::UserNotFound)?;
        let subject = RenameSubject {
            current_handle: old_handle.clone(),
            rsi_verified: rsi_verified_at.is_some(),
            last_handle_change_at,
        };
        let kind = evaluate(&subject, new_handle, now)?;

        if !old_handle.eq_ignore_ascii_case(new_handle) {
            let taken: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM users \
                 WHERE lower(claimed_handle) = lower($1) AND id <> $2)",
            )
            .bind(new_handle)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if taken {
                return Err(RenameRefusal::Taken.into());
            }
            let reserved: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM handle_renames \
                 WHERE lower(old_handle) = lower($1) AND reserved_until > $2 \
                 AND user_id <> $3)",
            )
            .bind(new_handle)
            .bind(now)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if reserved {
                return Err(RenameRefusal::Reserved.into());
            }
        }

        // The user row: new handle, verification reset, cooldown started,
        // and the chat account pinned to the localpart it already has.
        let updated = sqlx::query(
            "UPDATE users SET claimed_handle = $2, \
                 rsi_verified_at = NULL, rsi_verify_code = NULL, rsi_verify_expires_at = NULL, \
                 last_handle_change_at = $3, \
                 chat_localpart = COALESCE(chat_localpart, lower($4)), \
                 updated_at = $3 \
             WHERE id = $1",
        )
        .bind(user_id)
        .bind(new_handle)
        .bind(now)
        .bind(&old_handle)
        .execute(&mut *tx)
        .await;
        match updated {
            Err(e) if is_unique_violation(&e) => return Err(RenameRefusal::Taken.into()),
            r => {
                r?;
            }
        }

        // Each row keeps its own case convention: tables that store
        // handles lower-cased (several CHECK it) get the lower-cased new
        // handle, the rest get it as written.
        for (table, column) in HANDLE_COLUMNS {
            // Table and column names come from the fixed list above, never
            // from a caller; both handles are bound.
            let sql = format!(
                "UPDATE {table} SET {column} = \
                     CASE WHEN {column} = lower({column}) THEN lower($2) ELSE $2 END \
                 WHERE lower({column}) = lower($1)"
            );
            let res = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(&old_handle)
                .bind(new_handle)
                .execute(&mut *tx)
                .await;
            match res {
                // Rows already filed under the new handle, left by a
                // deleted account. Refuse rather than merge two people's data.
                Err(e) if is_unique_violation(&e) => return Err(RenameRefusal::Taken.into()),
                r => {
                    r?;
                }
            }
        }

        // friendships keeps each pair as (lower-sorted a, b); a rename can
        // put a pair out of order, and lookups rely on it.
        sqlx::query(
            "UPDATE friendships SET handle_a = handle_b, handle_b = handle_a \
             WHERE lower(handle_a) > lower(handle_b) \
               AND (lower(handle_a) = lower($1) OR lower(handle_b) = lower($1))",
        )
        .bind(new_handle)
        .execute(&mut *tx)
        .await?;

        let rename: RenameRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO handle_renames \
                 (id, user_id, old_handle, new_handle, was_verified, free_change, \
                  renamed_at, reserved_until) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             RETURNING {RENAME_COLUMNS}"
        )))
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(&old_handle)
        .bind(new_handle)
        .bind(subject.rsi_verified)
        .bind(kind == RenameKind::Free)
        .bind(now)
        .bind(now + RESERVATION)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(rename_from_row(rename))
    }

    async fn redirect_for(
        &self,
        old_handle: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, HandleRenameError> {
        // A live account holding the name wins, including a user who
        // renamed away and back again.
        Ok(sqlx::query_scalar(
            "SELECT u.claimed_handle FROM handle_renames r \
             JOIN users u ON u.id = r.user_id \
             WHERE lower(r.old_handle) = lower($1) AND r.reserved_until > $2 \
               AND NOT EXISTS (SELECT 1 FROM users h \
                               WHERE lower(h.claimed_handle) = lower($1)) \
             ORDER BY r.reserved_until DESC LIMIT 1",
        )
        .bind(old_handle)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn latest_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Option<HandleRename>, HandleRenameError> {
        let row: Option<RenameRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {RENAME_COLUMNS} FROM handle_renames \
             WHERE user_id = $1 ORDER BY renamed_at DESC LIMIT 1"
        )))
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(rename_from_row))
    }

    async fn pending_spicedb_moves(
        &self,
        limit: i64,
    ) -> Result<Vec<HandleRename>, HandleRenameError> {
        let rows: Vec<RenameRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {RENAME_COLUMNS} FROM handle_renames \
             WHERE spicedb_moved_at IS NULL ORDER BY renamed_at LIMIT $1"
        )))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(rename_from_row).collect())
    }

    async fn mark_spicedb_moved(
        &self,
        id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), HandleRenameError> {
        sqlx::query("UPDATE handle_renames SET spicedb_moved_at = $2 WHERE id = $1")
            .bind(id)
            .bind(at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn recent(&self, since: DateTime<Utc>) -> Result<Vec<HandleRename>, HandleRenameError> {
        let rows: Vec<RenameRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {RENAME_COLUMNS} FROM handle_renames \
             WHERE renamed_at >= $1 ORDER BY renamed_at"
        )))
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(rename_from_row).collect())
    }
}

#[cfg(test)]
pub mod test_support {
    //! In-memory store for route tests. Shares a `MemoryUserStore` so a
    //! rename is visible through `find_by_id` / `find_by_handle`, the way
    //! the Postgres impl's transaction makes it visible through `users`.

    use super::*;
    use crate::users::test_support::MemoryUserStore;
    use crate::users::UserStore;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    pub struct MemoryHandleRenameStore {
        users: Arc<MemoryUserStore>,
        last_change: Mutex<HashMap<Uuid, DateTime<Utc>>>,
        renames: Mutex<Vec<HandleRename>>,
    }

    impl MemoryHandleRenameStore {
        pub fn new(users: Arc<MemoryUserStore>) -> Self {
            Self {
                users,
                last_change: Mutex::new(HashMap::new()),
                renames: Mutex::new(Vec::new()),
            }
        }

        /// The user holding a reservation on `handle` at `now`, if any.
        pub async fn reserved_by(
            &self,
            handle: &str,
            now: DateTime<Utc>,
        ) -> Result<Option<Uuid>, HandleRenameError> {
            Ok(self
                .renames
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|r| r.old_handle.eq_ignore_ascii_case(handle) && r.reserved_until > now)
                .map(|r| r.user_id))
        }
    }

    #[async_trait]
    impl HandleRenameStore for MemoryHandleRenameStore {
        async fn rename(
            &self,
            user_id: Uuid,
            new_handle: &str,
            now: DateTime<Utc>,
        ) -> Result<HandleRename, HandleRenameError> {
            let user = self
                .users
                .find_by_id(user_id)
                .await
                .map_err(|_| HandleRenameError::UserNotFound)?
                .ok_or(HandleRenameError::UserNotFound)?;
            let subject = RenameSubject {
                current_handle: user.claimed_handle.clone(),
                rsi_verified: user.rsi_verified_at.is_some(),
                last_handle_change_at: self.last_change.lock().unwrap().get(&user_id).copied(),
            };
            let kind = evaluate(&subject, new_handle, now)?;

            // Availability. A change of case only keeps the same name.
            if !user.claimed_handle.eq_ignore_ascii_case(new_handle) {
                if let Ok(Some(other)) = self.users.find_by_handle(new_handle).await {
                    if other.id != user_id {
                        return Err(RenameRefusal::Taken.into());
                    }
                }
                if let Some(holder) = self.reserved_by(new_handle, now).await? {
                    if holder != user_id {
                        return Err(RenameRefusal::Reserved.into());
                    }
                }
            }

            self.users
                .apply_handle_rename(user_id, new_handle)
                .ok_or(HandleRenameError::UserNotFound)?;
            self.last_change.lock().unwrap().insert(user_id, now);
            let rename = HandleRename {
                id: Uuid::new_v4(),
                user_id,
                old_handle: user.claimed_handle,
                new_handle: new_handle.to_owned(),
                was_verified: subject.rsi_verified,
                free_change: kind == RenameKind::Free,
                renamed_at: now,
                reserved_until: now + RESERVATION,
                spicedb_moved_at: None,
            };
            self.renames.lock().unwrap().push(rename.clone());
            Ok(rename)
        }

        async fn redirect_for(
            &self,
            old_handle: &str,
            now: DateTime<Utc>,
        ) -> Result<Option<String>, HandleRenameError> {
            // A live account holding the name wins: that includes a user
            // who renamed away and back again.
            if let Ok(Some(_)) = self.users.find_by_handle(old_handle).await {
                return Ok(None);
            }
            let Some(user_id) = self.reserved_by(old_handle, now).await? else {
                return Ok(None);
            };
            Ok(self
                .users
                .find_by_id(user_id)
                .await
                .ok()
                .flatten()
                .map(|u| u.claimed_handle))
        }

        async fn latest_for_user(
            &self,
            user_id: Uuid,
        ) -> Result<Option<HandleRename>, HandleRenameError> {
            Ok(self
                .renames
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|r| r.user_id == user_id)
                .cloned())
        }

        async fn pending_spicedb_moves(
            &self,
            limit: i64,
        ) -> Result<Vec<HandleRename>, HandleRenameError> {
            Ok(self
                .renames
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.spicedb_moved_at.is_none())
                .take(usize::try_from(limit).unwrap_or(0))
                .cloned()
                .collect())
        }

        async fn mark_spicedb_moved(
            &self,
            id: Uuid,
            at: DateTime<Utc>,
        ) -> Result<(), HandleRenameError> {
            if let Some(r) = self.renames.lock().unwrap().iter_mut().find(|r| r.id == id) {
                r.spicedb_moved_at = Some(at);
            }
            Ok(())
        }

        async fn recent(
            &self,
            since: DateTime<Utc>,
        ) -> Result<Vec<HandleRename>, HandleRenameError> {
            Ok(self
                .renames
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.renamed_at >= since)
                .cloned()
                .collect())
        }
    }
}

#[cfg(test)]
mod postgres_tests {
    //! Against the real schema. Skipped without STARSTATS_TEST_DATABASE_URL.

    use super::*;

    /// Columns that hold a handle but are deliberately not rewritten;
    /// [`HANDLE_COLUMNS`] says why for each.
    const NOT_REWRITTEN: &[(&str, &str)] = &[
        ("users", "claimed_handle"),
        ("audit_log", "actor_handle"),
        ("handle_renames", "old_handle"),
        ("handle_renames", "new_handle"),
        ("chat_rooms", "dm_a"),
        ("chat_rooms", "dm_b"),
        ("chat_room_members", "handle"),
    ];

    async fn pool() -> Option<sqlx::PgPool> {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres rename test");
            return None;
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
        Some(pool)
    }

    /// Every text column whose name says it holds a handle, or that is one
    /// of the known moderator-attribution columns, must be listed as either
    /// rewritten or deliberately not. A new handle column fails here until
    /// someone decides which.
    #[tokio::test]
    async fn every_handle_column_is_accounted_for() {
        let Some(pool) = pool().await else { return };
        let cols: Vec<(String, String)> = sqlx::query_as(
            "SELECT table_name::text, column_name::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND data_type = 'text' \
               AND (column_name LIKE '%handle%' OR column_name IN \
                    ('resolved_by','removed_by','created_by','published_by', \
                     'acknowledged_by','restricted_by','dm_a','dm_b'))",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let unaccounted: Vec<_> = cols
            .iter()
            .filter(|(t, c)| {
                let pair = (t.as_str(), c.as_str());
                !HANDLE_COLUMNS.contains(&pair) && !NOT_REWRITTEN.contains(&pair)
            })
            .collect();
        assert!(
            unaccounted.is_empty(),
            "handle columns neither rewritten nor excluded: {unaccounted:?}"
        );
    }

    #[tokio::test]
    async fn rename_leaves_no_row_on_the_old_handle() {
        let Some(pool) = pool().await else { return };
        let tag = &Uuid::new_v4().simple().to_string()[..8];
        let (old, new, friend) = (
            format!("rnaOld{tag}"),
            format!("rnzNew{tag}"),
            // Sorts between the two lower-cased handles: the pair starts as
            // (old, friend) and must become (friend, new) after the rename.
            format!("rnmid{tag}"),
        );
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, claimed_handle, rsi_verified_at) \
             VALUES ($1, $2, 'phc', $3, now())",
        )
        .bind(user_id)
        .bind(format!("{tag}@example.com"))
        .bind(&old)
        .execute(&pool)
        .await
        .unwrap();
        // Mixed storage conventions: lower-cased and as-written.
        sqlx::query("INSERT INTO friendships (handle_a, handle_b) VALUES ($1, $2)")
            .bind(old.to_lowercase())
            .bind(&friend)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO user_blocks (blocker_handle, blocked_handle) VALUES ($1, $2)")
            .bind(&old)
            .bind(&friend)
            .execute(&pool)
            .await
            .unwrap();

        let store = PostgresHandleRenameStore::new(pool.clone());
        let r = store.rename(user_id, &new, Utc::now()).await.unwrap();
        assert_eq!(r.old_handle, old);
        assert!(r.was_verified);

        for (table, column) in HANDLE_COLUMNS {
            let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table} WHERE lower({column}) = lower($1)"
            )))
            .bind(&old)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(n, 0, "{table}.{column} still holds the old handle");
        }

        let pair: (String, String) = sqlx::query_as(
            "SELECT handle_a, handle_b FROM friendships WHERE lower(handle_b) = lower($1) \
                OR lower(handle_a) = lower($1)",
        )
        .bind(&new)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pair, (friend.clone(), new.to_lowercase()));
        let blocker: String =
            sqlx::query_scalar("SELECT blocker_handle FROM user_blocks WHERE blocked_handle = $1")
                .bind(&friend)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            blocker, new,
            "an as-written row keeps the new handle as written"
        );

        let (verified, localpart): (Option<DateTime<Utc>>, Option<String>) =
            sqlx::query_as("SELECT rsi_verified_at, chat_localpart FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(verified.is_none());
        assert_eq!(localpart, Some(old.to_lowercase()));
        assert_eq!(
            store.redirect_for(&old, Utc::now()).await.unwrap(),
            Some(new.clone())
        );

        // Nobody else can sign up under the held handle, in any case.
        use crate::users::{PostgresUserStore, UserError, UserStore};
        let users = PostgresUserStore::new(pool.clone());
        let signup = users
            .create(
                &format!("squat{tag}@example.com"),
                "phc",
                &old.to_uppercase(),
            )
            .await;
        assert!(matches!(signup, Err(UserError::HandleTaken)), "{signup:?}");
    }

    /// The game log names the player as they were called at the time, so a
    /// kill from before the rename has the OLD handle in `payload.killer`.
    /// Stats must still count it.
    #[tokio::test]
    async fn kills_logged_under_the_old_handle_still_count() {
        use crate::repo::{EventQuery, PostgresStore};
        let Some(pool) = pool().await else { return };
        let tag = &Uuid::new_v4().simple().to_string()[..8];
        let (old, new) = (format!("ksOld{tag}"), format!("ksNew{tag}"));
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, claimed_handle) \
             VALUES ($1, $2, 'phc', $3)",
        )
        .bind(user_id)
        .bind(format!("ks{tag}@example.com"))
        .bind(&old)
        .execute(&pool)
        .await
        .unwrap();
        // One kill, one death and one self-kill, all logged before the rename.
        for (key, killer, victim) in [
            ("kill", old.as_str(), "NPC_Pirate_123"),
            ("death", "NPC_Pirate_456", old.as_str()),
            ("self", old.as_str(), old.as_str()),
        ] {
            sqlx::query(
                "INSERT INTO events (id, idempotency_key, claimed_handle, event_type, \
                                     event_timestamp, log_source, source_offset, raw_line, payload) \
                 VALUES (gen_random_uuid(), $1, lower($2), 'actor_death', now(), 'live', 0, '', \
                         jsonb_build_object('killer', $3::text, 'victim', $4::text, \
                                            'zone', 'Zone_' || $1))",
            )
            .bind(format!("{key}-{tag}"))
            .bind(&old)
            .bind(killer)
            .bind(victim)
            .execute(&pool)
            .await
            .unwrap();
        }

        PostgresHandleRenameStore::new(pool.clone())
            .rename(user_id, &new, Utc::now())
            .await
            .unwrap();

        let store = PostgresStore::new(pool.clone());
        let (kills, deaths, _) = store.combat_counts(&new, &new, None).await.unwrap();
        assert_eq!(
            kills, 1,
            "the pre-rename kill counts; the self-kill does not"
        );
        assert_eq!(deaths, 2, "the pre-rename death and the self-kill");
        let enemies = store
            .kill_enemy_breakdown(&new, &new, None, 10)
            .await
            .unwrap();
        assert_eq!(enemies.len(), 1);
        assert_eq!(enemies[0].value, "NPC_Pirate");

        // Deaths by zone go through the generic victim filter.
        let zones = store
            .payload_field_breakdown(
                &new,
                "actor_death",
                "zone",
                Some(crate::repo::PayloadFilter {
                    field: "victim",
                    equals: &new,
                }),
                None,
                None,
                10,
            )
            .await
            .unwrap();
        assert_eq!(zones.len(), 2, "both pre-rename deaths: {zones:?}");
    }
}

#[cfg(test)]
mod store_tests {
    use super::test_support::MemoryHandleRenameStore;
    use super::*;
    use crate::users::test_support::MemoryUserStore;
    use crate::users::UserStore;
    use std::sync::Arc;

    async fn fixture() -> (Arc<MemoryUserStore>, MemoryHandleRenameStore, Uuid) {
        let users = Arc::new(MemoryUserStore::new());
        let u = users
            .create("pilot@example.com", "phc", "OldPilot")
            .await
            .unwrap();
        let store = MemoryHandleRenameStore::new(users.clone());
        (users, store, u.id)
    }

    #[tokio::test]
    async fn rename_moves_the_handle_and_resets_verification() {
        let (users, store, id) = fixture().await;
        users.mark_rsi_verified(id).await.unwrap();
        let now = Utc::now();
        let r = store.rename(id, "NewPilot", now).await.unwrap();
        assert_eq!(r.old_handle, "OldPilot");
        assert!(r.was_verified);
        assert!(!r.free_change);
        assert_eq!(r.reserved_until, now + RESERVATION);
        let u = users.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(u.claimed_handle, "NewPilot");
        assert!(u.rsi_verified_at.is_none());
    }

    #[tokio::test]
    async fn free_change_is_used_once_then_the_cooldown_applies() {
        let (_users, store, id) = fixture().await;
        let now = Utc::now();
        let first = store.rename(id, "NewPilot", now).await.unwrap();
        assert!(first.free_change);
        let second = store
            .rename(id, "ThirdPilot", now + Duration::days(1))
            .await;
        assert!(matches!(
            second,
            Err(HandleRenameError::Refused(RenameRefusal::Cooldown { .. }))
        ));
        let later = store
            .rename(id, "ThirdPilot", now + COOLDOWN + Duration::seconds(1))
            .await
            .unwrap();
        assert!(!later.free_change);
    }

    #[tokio::test]
    async fn a_reserved_handle_cannot_be_taken_by_another_account() {
        let (users, store, id) = fixture().await;
        let now = Utc::now();
        store.rename(id, "NewPilot", now).await.unwrap();
        let other = users
            .create("other@example.com", "phc", "Someone")
            .await
            .unwrap();
        let attempt = store.rename(other.id, "oldpilot", now).await;
        assert!(matches!(
            attempt,
            Err(HandleRenameError::Refused(RenameRefusal::Reserved))
        ));
        assert_eq!(store.reserved_by("OLDPILOT", now).await.unwrap(), Some(id));
        let after = now + RESERVATION + Duration::seconds(1);
        assert_eq!(store.reserved_by("OldPilot", after).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_handle_held_by_a_live_account_is_taken() {
        let (users, store, id) = fixture().await;
        users
            .create("other@example.com", "phc", "Someone")
            .await
            .unwrap();
        let attempt = store.rename(id, "someone", Utc::now()).await;
        assert!(matches!(
            attempt,
            Err(HandleRenameError::Refused(RenameRefusal::Taken))
        ));
    }

    #[tokio::test]
    async fn the_old_handle_redirects_until_the_reservation_ends() {
        let (_users, store, id) = fixture().await;
        let now = Utc::now();
        store.rename(id, "NewPilot", now).await.unwrap();
        assert_eq!(
            store
                .redirect_for("oldpilot", now)
                .await
                .unwrap()
                .as_deref(),
            Some("NewPilot")
        );
        let after = now + RESERVATION + Duration::seconds(1);
        assert_eq!(store.redirect_for("OldPilot", after).await.unwrap(), None);
    }

    #[tokio::test]
    async fn renaming_back_to_your_own_reserved_handle_is_allowed_and_ends_the_redirect() {
        let (_users, store, id) = fixture().await;
        let now = Utc::now();
        store.rename(id, "NewPilot", now).await.unwrap();
        let back = now + COOLDOWN + Duration::seconds(1);
        store.rename(id, "OldPilot", back).await.unwrap();
        assert_eq!(store.redirect_for("OldPilot", back).await.unwrap(), None);
    }

    #[tokio::test]
    async fn spicedb_moves_queue_until_marked() {
        let (_users, store, id) = fixture().await;
        let r = store.rename(id, "NewPilot", Utc::now()).await.unwrap();
        assert_eq!(
            store.pending_spicedb_moves(10).await.unwrap(),
            vec![r.clone()]
        );
        store.mark_spicedb_moved(r.id, Utc::now()).await.unwrap();
        assert!(store.pending_spicedb_moves(10).await.unwrap().is_empty());
        assert_eq!(
            store.latest_for_user(id).await.unwrap().map(|x| x.id),
            Some(r.id)
        );
    }

    /// Fails while `down` is set, then records each move it makes.
    struct FakeMover {
        down: std::sync::atomic::AtomicBool,
        moves: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl HandleMover for FakeMover {
        async fn move_handle(&self, old: &str, new: &str) -> anyhow::Result<usize> {
            if self.down.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("SpiceDB unavailable");
            }
            self.moves.lock().unwrap().push((old.into(), new.into()));
            Ok(1)
        }
    }

    #[tokio::test]
    async fn the_reconciler_retries_a_move_until_it_lands() {
        let (_users, store, id) = fixture().await;
        store.rename(id, "NewPilot", Utc::now()).await.unwrap();
        let mover = FakeMover {
            down: std::sync::atomic::AtomicBool::new(true),
            moves: std::sync::Mutex::new(Vec::new()),
        };
        assert_eq!(reconcile_spicedb_moves(&store, &mover).await.unwrap(), 0);
        assert_eq!(store.pending_spicedb_moves(10).await.unwrap().len(), 1);

        mover.down.store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(reconcile_spicedb_moves(&store, &mover).await.unwrap(), 1);
        assert!(store.pending_spicedb_moves(10).await.unwrap().is_empty());
        assert_eq!(
            *mover.moves.lock().unwrap(),
            vec![("OldPilot".to_string(), "NewPilot".to_string())]
        );
        // Nothing left: a further pass moves nothing.
        assert_eq!(reconcile_spicedb_moves(&store, &mover).await.unwrap(), 0);
    }
}

#[cfg(test)]
mod index_tests {
    use super::*;

    fn rename(user_id: Uuid, old: &str, new: &str, at: DateTime<Utc>) -> HandleRename {
        HandleRename {
            id: Uuid::new_v4(),
            user_id,
            old_handle: old.into(),
            new_handle: new.into(),
            was_verified: false,
            free_change: false,
            renamed_at: at,
            reserved_until: at + RESERVATION,
            spicedb_moved_at: None,
        }
    }

    #[test]
    fn a_token_from_before_the_rename_acts_as_the_new_handle() {
        let index = HandleIndex::new();
        let id = Uuid::new_v4();
        let at = Utc::now();
        index.record(&rename(id, "OldPilot", "NewPilot", at));
        let before = usize::try_from(at.timestamp() - 60).unwrap();
        assert_eq!(
            index.resolve(id, "oldpilot", before).as_deref(),
            Some("NewPilot")
        );
        // Same second as the rename still counts as before it.
        let same = usize::try_from(at.timestamp()).unwrap();
        assert_eq!(
            index.resolve(id, "OldPilot", same).as_deref(),
            Some("NewPilot")
        );
    }

    #[test]
    fn a_token_already_on_the_new_handle_or_another_account_is_untouched() {
        let index = HandleIndex::new();
        let id = Uuid::new_v4();
        let at = Utc::now();
        index.record(&rename(id, "OldPilot", "NewPilot", at));
        let t = usize::try_from(at.timestamp()).unwrap();
        assert_eq!(index.resolve(id, "NewPilot", t), None);
        assert_eq!(index.resolve(Uuid::new_v4(), "OldPilot", t), None);
        // A token minted after the rename is trusted as it stands.
        assert_eq!(index.resolve(id, "OldPilot", t + 5), None);
    }

    #[test]
    fn two_renames_resolve_to_the_latest_and_keep_both_old_handles() {
        let index = HandleIndex::new();
        let id = Uuid::new_v4();
        let t0 = Utc::now() - Duration::days(40);
        let t1 = Utc::now();
        index.replace_all(&[
            rename(id, "First", "Second", t0),
            rename(id, "Second", "Third", t1),
        ]);
        let acct = index.get(id).unwrap();
        assert_eq!(acct.current_handle, "Third");
        assert_eq!(
            acct.previous_handles,
            vec!["Second".to_string(), "First".to_string()]
        );
        let old = usize::try_from(t0.timestamp() - 1).unwrap();
        assert_eq!(index.resolve(id, "First", old).as_deref(), Some("Third"));
    }

    #[test]
    fn renaming_back_drops_the_name_from_previous_handles() {
        let index = HandleIndex::new();
        let id = Uuid::new_v4();
        let t0 = Utc::now() - Duration::days(40);
        index.record(&rename(id, "A", "B", t0));
        index.record(&rename(id, "B", "A", Utc::now()));
        assert_eq!(
            index.get(id).unwrap().previous_handles,
            vec!["B".to_string()]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(verified: bool, last: Option<DateTime<Utc>>) -> RenameSubject {
        RenameSubject {
            current_handle: "OldPilot".into(),
            rsi_verified: verified,
            last_handle_change_at: last,
        }
    }

    #[test]
    fn never_verified_never_renamed_gets_the_free_change() {
        let now = Utc::now();
        assert_eq!(
            evaluate(&subject(false, None), "NewPilot", now),
            Ok(RenameKind::Free)
        );
    }

    #[test]
    fn verified_first_rename_is_standard() {
        let now = Utc::now();
        assert_eq!(
            evaluate(&subject(true, None), "NewPilot", now),
            Ok(RenameKind::Standard)
        );
    }

    #[test]
    fn second_rename_inside_cooldown_is_refused_even_when_unverified() {
        let now = Utc::now();
        let last = now - Duration::days(3);
        assert_eq!(
            evaluate(&subject(false, Some(last)), "NewPilot", now),
            Err(RenameRefusal::Cooldown {
                until: last + COOLDOWN
            })
        );
    }

    #[test]
    fn rename_after_cooldown_is_standard() {
        let now = Utc::now();
        let last = now - COOLDOWN;
        assert_eq!(
            evaluate(&subject(true, Some(last)), "NewPilot", now),
            Ok(RenameKind::Standard)
        );
    }

    #[test]
    fn same_handle_is_refused_but_a_case_change_is_allowed() {
        let now = Utc::now();
        assert_eq!(
            evaluate(&subject(true, None), "OldPilot", now),
            Err(RenameRefusal::Unchanged)
        );
        assert_eq!(
            evaluate(&subject(true, None), "oldpilot", now),
            Ok(RenameKind::Standard)
        );
    }

    #[test]
    fn invalid_handle_is_refused_before_anything_else() {
        let now = Utc::now();
        assert_eq!(
            evaluate(&subject(false, None), "bad handle!", now),
            Err(RenameRefusal::InvalidHandle)
        );
    }
}
