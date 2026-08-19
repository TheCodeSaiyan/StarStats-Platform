//! User accounts for first-party auth.
//!
//! Stores the email + password-hash + claimed RSI handle. Hashing is
//! Argon2id with the `argon2` crate's defaults (currently OWASP's
//! recommended t=2, m=19456, p=1 baseline). Verification reads the
//! parameters out of the PHC string so we can raise cost later
//! without breaking existing accounts.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    /// Surfaced by the profile endpoints in a future slice.
    pub email: String,
    pub password_hash: String,
    pub claimed_handle: String,
    /// Surfaced by the profile endpoints in a future slice.
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    /// `Some(ts)` once the user has clicked the verification link.
    /// Surfaced by the profile endpoints in a future slice; today
    /// only the verify-email handler reads/writes it.
    #[allow(dead_code)]
    pub email_verified_at: Option<DateTime<Utc>>,
    /// Expiry of the currently-issued verification token, if any.
    /// `find_by_verification_token` populates this so the handler can
    /// distinguish "unknown token" from "expired token" without a
    /// second round trip. `None` everywhere else (find_by_email /
    /// find_by_id deliberately don't read this — handlers that don't
    /// care should leave the field as `None`).
    pub email_verification_expires_at: Option<DateTime<Utc>>,
    /// Last time the password hash was rotated. Tracked for the
    /// JWT-iat session-revocation check (a follow-up — currently
    /// device tokens are revoked immediately on reset, and user
    /// JWTs expire naturally within their 1-hour TTL). Always
    /// populated; the migration's `DEFAULT NOW()` covers existing
    /// rows.
    #[allow(dead_code)]
    pub password_changed_at: DateTime<Utc>,
    /// Email address the user has staged but not yet confirmed. Login
    /// still uses `email` while this is `Some(_)`; once the user
    /// clicks the verification link, the email-change-verify handler
    /// promotes `pending_email` -> `email` atomically.
    pub pending_email: Option<String>,
    /// Expiry of the password-reset token. Smuggled here only by
    /// `find_by_password_reset_token`; `None` on all other reads.
    /// Mirrors the smuggle pattern used for email verification.
    pub password_reset_expires_at: Option<DateTime<Utc>>,
    /// Expiry of the pending-email-change token. Smuggled here only
    /// by `find_by_pending_email_token`; `None` on all other reads.
    pub pending_email_expires_at: Option<DateTime<Utc>>,
    /// `Some(ts)` once the user has proven ownership of `claimed_handle`
    /// by pasting the verification code into their RSI public-profile
    /// bio. Public profiles + org shares are gated on this — without
    /// proof, anyone could sign up as `TheCodeSaiyan` and publish under
    /// that name.
    pub rsi_verified_at: Option<DateTime<Utc>>,
    /// The currently-issued RSI bio verification code, if any. The
    /// user pastes this into their RSI profile and we check the page
    /// for it. Cleared on successful verification.
    pub rsi_verify_code: Option<String>,
    /// Expiry of the currently-issued RSI verification code. Always
    /// populated alongside `rsi_verify_code`, because they're issued
    /// as a pair.
    pub rsi_verify_expires_at: Option<DateTime<Utc>>,
    /// `Some(ts)` once the user has completed TOTP setup (paired an
    /// authenticator app AND submitted a valid code). Login enforces
    /// the second factor on this flag — `totp_setup_at` set without
    /// `totp_enabled_at` means setup is in flight but not yet active,
    /// and login still works with password alone in that state.
    /// The encrypted secret + nonce live on the row but are read via
    /// `UserStore::get_totp_secret` on demand so the hot login path
    /// doesn't pull bytea every request.
    pub totp_enabled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum UserError {
    #[error("email already registered")]
    EmailTaken,
    #[error("handle already claimed")]
    HandleTaken,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("password hashing failed: {0}")]
    Hash(String),
}

/// RSI handles are ASCII alphanumeric + underscore + hyphen, ≤ 64 chars.
/// Enforced at every boundary that accepts a user-supplied handle —
/// signup, sharing recipients, RSI profile / org lookups — to keep
/// downstream renderers, SpiceDB lookups, and URL builders working on
/// a small, predictable alphabet.
// M-S3: single source of truth in `crate::validation`. Re-exported here so
// the four modules that import `crate::users::validate_handle` keep working.
pub use crate::validation::validate_handle;

/// Filter shape for the admin `list_users` call. Empty filters
/// return the most recently created users up to `limit`.
#[derive(Debug, Clone, Default)]
pub struct ListUsersFilters {
    /// Case-insensitive substring match against `claimed_handle`
    /// OR `email`. Admins reason about either depending on context
    /// so a single field that matches both is more useful than
    /// two separate filters.
    pub q: Option<String>,
    /// Page size, clamped by the handler to [1, 200].
    pub limit: i64,
    /// Offset for pagination.
    pub offset: i64,
}

#[async_trait]
pub trait UserStore: Send + Sync + 'static {
    async fn create(
        &self,
        email: &str,
        password_hash: &str,
        claimed_handle: &str,
    ) -> Result<User, UserError>;
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, UserError>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, UserError>;
    /// Look up a user by their claimed RSI handle (case-insensitive).
    /// Used by the sharing endpoints to validate that a recipient
    /// exists before mutating SpiceDB.
    async fn find_by_handle(&self, handle: &str) -> Result<Option<User>, UserError>;

    /// Admin-only paginated list. Ordered by `created_at DESC` so
    /// new signups surface first. Substring search runs over
    /// `claimed_handle` OR `email` (case-insensitive).
    async fn list_users(&self, filters: ListUsersFilters) -> Result<Vec<User>, UserError>;

    // -- Email verification ------------------------------------------
    /// Stash a freshly-minted verification token + its expiry on the
    /// user row. Best-effort from the caller's perspective: a failure
    /// here logs and is swallowed so signup still succeeds.
    async fn set_verification_token(
        &self,
        user_id: Uuid,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), UserError>;

    /// Look up a user by their pending verification token. Returns
    /// `None` for unknown tokens. The handler itself enforces expiry —
    /// the lookup ignores the timestamp so the error path can return
    /// "invalid_or_expired" without two round trips.
    async fn find_by_verification_token(&self, token: &str) -> Result<Option<User>, UserError>;

    /// Mark verification complete: set `email_verified_at = now()` and
    /// clear the token columns so the same link can't be replayed.
    async fn mark_email_verified(&self, user_id: Uuid) -> Result<(), UserError>;

    // -- Account management ------------------------------------------
    /// Replace the password hash for `user_id` and bump
    /// `password_changed_at` to NOW(). Caller is responsible for
    /// verifying the existing password and re-hashing the new one —
    /// this method is a blind UPDATE.
    async fn update_password(&self, user_id: Uuid, new_phc: &str) -> Result<(), UserError>;

    /// Hard-delete a user row. The Postgres impl runs DELETE inside a
    /// transaction; the FK on `devices.user_id` is `ON DELETE CASCADE`
    /// so paired devices are removed atomically. Returns Ok even if no
    /// row matched (idempotent — the caller already authenticated as
    /// this user, so a missing row implies a concurrent delete).
    async fn delete_user(&self, user_id: Uuid) -> Result<(), UserError>;

    /// Admin-only hard delete: like [`UserStore::delete_user`], but the
    /// user's events are DELETED rather than pseudonymised, and the
    /// derived per-handle tables go with them.
    ///
    /// `delete_user` leaves `session_summary`, `character_records` and
    /// `stat_event_counts` behind -- its own comment records that gap.
    /// Purge closes it, because "purge" that leaves derived rows keyed
    /// to the handle would be a promise the function does not keep.
    /// (`stat_event_counts` would self-heal on the nightly reconcile;
    /// the other two would not.)
    ///
    /// Idempotent: purging an already-gone user is a no-op.
    async fn purge_user(&self, user_id: Uuid) -> Result<(), UserError>;

    // -- Password reset ----------------------------------------------
    /// Stash a fresh password-reset token + expiry on the user row.
    /// Overwrites any prior token (last-write-wins is the right
    /// posture: a user who clicks "Forgot password" twice expects the
    /// most recent email to be the live one).
    async fn set_password_reset_token(
        &self,
        user_id: Uuid,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), UserError>;

    /// Look up a user by their pending password-reset token. Returns
    /// `None` for unknown tokens. The handler enforces expiry via the
    /// `password_reset_expires_at` field smuggled onto the returned
    /// `User`.
    async fn find_by_password_reset_token(&self, token: &str) -> Result<Option<User>, UserError>;

    /// Atomically: update the password hash, clear the reset-token
    /// columns, and bump `password_changed_at`. Used by the
    /// reset-complete handler. Returns Ok even if the row vanished
    /// between lookup and complete (idempotent).
    async fn complete_password_reset(&self, user_id: Uuid, new_phc: &str) -> Result<(), UserError>;

    // -- Email change ------------------------------------------------
    /// Stage a new email address with a verification token + expiry.
    /// Login continues to use the existing `email` until the token
    /// is redeemed. Caller has already validated the email shape and
    /// confirmed the new address is not already taken.
    async fn set_pending_email(
        &self,
        user_id: Uuid,
        new_email: &str,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), UserError>;

    /// Look up a user by their pending-email-change token. The
    /// returned `User` has `pending_email_expires_at` populated for
    /// the handler's expiry check.
    async fn find_by_pending_email_token(&self, token: &str) -> Result<Option<User>, UserError>;

    /// Atomically: copy `pending_email` -> `email`, clear pending
    /// columns, and set `email_verified_at = NOW()` (the new address
    /// is verified by the click-through). Returns
    /// `UserError::EmailTaken` if the new address has been claimed
    /// by another user since the change was staged.
    async fn commit_pending_email(&self, user_id: Uuid) -> Result<(), UserError>;

    // -- RSI handle verification -------------------------------------
    /// Stash a fresh RSI verification code + expiry on the user row.
    /// Last-write-wins: re-issuing replaces any prior code, which is
    /// the right posture if a user starts the flow twice.
    async fn set_rsi_verify_code(
        &self,
        user_id: Uuid,
        code: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), UserError>;

    /// Mark the RSI handle proven: set `rsi_verified_at = NOW()` and
    /// clear the code / expiry columns so the user can take their
    /// bio back without leaving stale state behind.
    async fn mark_rsi_verified(&self, user_id: Uuid) -> Result<(), UserError>;

    // -- TOTP 2FA ----------------------------------------------------
    /// Stash an encrypted TOTP shared secret + nonce on the user row
    /// and bump `totp_setup_at`. `totp_enabled_at` is left untouched
    /// because the user hasn't proven the secret yet — login still
    /// works with password alone until they confirm.
    async fn start_totp_setup(
        &self,
        user_id: Uuid,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<(), UserError>;

    /// Read the encrypted secret + nonce previously stashed by
    /// `start_totp_setup`. Returns `None` when no setup has been
    /// started; used by `confirm` and `verify-login`.
    async fn get_totp_secret(&self, user_id: Uuid)
        -> Result<Option<(Vec<u8>, Vec<u8>)>, UserError>;

    /// Mark TOTP enabled: set `totp_enabled_at = NOW()`. The secret
    /// + nonce stay on the row (they're needed for every login from
    /// here on). Login enforcement keys off this column.
    async fn mark_totp_enabled(&self, user_id: Uuid) -> Result<(), UserError>;

    /// Wipe TOTP state entirely: clear ciphertext, nonce,
    /// `totp_setup_at`, and `totp_enabled_at`. The `recovery_codes`
    /// rows for this user are deleted by the caller via
    /// [`RecoveryCodeStore::clear_for_user`] — this method handles
    /// the user row only.
    async fn disable_totp(&self, user_id: Uuid) -> Result<(), UserError>;

    // -- Audit v2.1 §C — abuse-signal auto-pause -----------------------
    //
    // Tiny pair of helpers backing the cross-report-cluster pause: the
    // report handler stamps `shares_paused_until` when the threshold
    // crosses, and the add_share handler reads it on every new grant.
    // Both work by handle so the report path can stamp the owner
    // without a separate id lookup, and the add_share path can gate
    // off `auth.preferred_username` directly.

    /// Returns the current `shares_paused_until` value for the owner
    /// of `handle`. `Ok(None)` covers BOTH "user does not exist" and
    /// "column is NULL" — the gate treats them the same (not paused).
    async fn get_shares_paused_until_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<DateTime<Utc>>, UserError>;

    /// Stamp `shares_paused_until = until` on the user identified by
    /// `handle`. Pass `None` to clear (admin-initiated unpause).
    /// Idempotent — unknown handles silently succeed with zero rows
    /// touched, mirroring the lookup contract above.
    async fn set_shares_paused_until_by_handle(
        &self,
        handle: &str,
        until: Option<DateTime<Utc>>,
    ) -> Result<(), UserError>;

    // -- Piece 4 — /discover listing opt-out -------------------------
    //
    // Public visibility itself is a SpiceDB relation; whether a public
    // profile is *also* surfaced on the browsable `/discover` listing
    // is a separate per-user boolean stored on this row. Read by the
    // discover route handler after it gets the public-profile handle
    // set from SpiceDB; written by the extended `/v1/me/visibility`
    // POST. Defaults to FALSE (appear in listing).
    //
    // Both methods key off the handle (rather than user_id) because
    // every existing caller already has the handle on hand: the
    // visibility endpoint reads `auth.preferred_username`, and the
    // discover handler walks the SpiceDB handle list. Avoiding an
    // intermediate id lookup keeps the hot paths to a single query.

    /// Returns the current `listing_opt_out` value for the owner of
    /// `handle`. `Ok(false)` covers BOTH "user does not exist" and
    /// "column is FALSE" — the discover filter treats them the same
    /// (eligible for listing, modulo the SpiceDB membership check
    /// that already pre-filtered the handle set).
    async fn get_listing_opt_out_by_handle(&self, handle: &str) -> Result<bool, UserError>;

    /// Stamp `listing_opt_out = value` on the user identified by
    /// `handle`. Idempotent — unknown handles silently succeed with
    /// zero rows touched, mirroring the lookup contract above.
    async fn set_listing_opt_out_by_handle(
        &self,
        handle: &str,
        value: bool,
    ) -> Result<(), UserError>;
}

// -- Argon2 helpers --------------------------------------------------

pub fn hash_password(password: &str) -> Result<String, UserError> {
    let salt = SaltString::generate(&mut OsRng);
    let hasher = Argon2::default();
    hasher
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| UserError::Hash(e.to_string()))
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

// -- Postgres impl ---------------------------------------------------

/// Always-populated user columns. Defines the column list every
/// `find_by_*` issues so adding a new persistent field is a one-line
/// change here rather than five.
const USER_SELECT: &str = "id, email, password_hash, claimed_handle, created_at, \
                           email_verified_at, password_changed_at, pending_email, \
                           rsi_verified_at, rsi_verify_code, rsi_verify_expires_at, \
                           totp_enabled_at";

type UserRow = (
    Uuid,
    String,
    String,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

/// Same column shape as [`UserRow`] but with one trailing
/// `Option<DateTime<Utc>>` for handlers that smuggle a per-token
/// expiry alongside the user row (verification, password reset,
/// pending email).
type UserRowWithExpiry = (
    Uuid,
    String,
    String,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

fn split_with_expiry(row: UserRowWithExpiry) -> (UserRow, Option<DateTime<Utc>>) {
    let (a, b, c, d, e, f, g, h, i, j, k, l, expires_at) = row;
    ((a, b, c, d, e, f, g, h, i, j, k, l), expires_at)
}

fn user_from_row(row: UserRow) -> User {
    let (
        id,
        email,
        password_hash,
        claimed_handle,
        created_at,
        email_verified_at,
        password_changed_at,
        pending_email,
        rsi_verified_at,
        rsi_verify_code,
        rsi_verify_expires_at,
        totp_enabled_at,
    ) = row;
    User {
        id,
        email,
        password_hash,
        claimed_handle,
        created_at,
        email_verified_at,
        email_verification_expires_at: None,
        password_changed_at,
        pending_email,
        password_reset_expires_at: None,
        pending_email_expires_at: None,
        rsi_verified_at,
        rsi_verify_code,
        rsi_verify_expires_at,
        totp_enabled_at,
    }
}

pub struct PostgresUserStore {
    pool: PgPool,
}

impl PostgresUserStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserStore for PostgresUserStore {
    async fn create(
        &self,
        email: &str,
        password_hash: &str,
        claimed_handle: &str,
    ) -> Result<User, UserError> {
        let id = Uuid::new_v4();
        // The two unique indexes (lower(email), lower(claimed_handle))
        // give us free contention checks. Map their constraint codes
        // back into typed errors so handlers can return 409 with a
        // useful body instead of 500.
        let sql = format!(
            "INSERT INTO users (id, email, password_hash, claimed_handle) \
             VALUES ($1, lower($2), $3, $4) \
             RETURNING {USER_SELECT}"
        );
        let row: Result<UserRow, sqlx::Error> = sqlx::query_as(&sql)
            .bind(id)
            .bind(email)
            .bind(password_hash)
            .bind(claimed_handle)
            .fetch_one(&self.pool)
            .await;

        match row {
            Ok(row) => Ok(user_from_row(row)),
            Err(sqlx::Error::Database(db)) if db.constraint() == Some("users_email_uq") => {
                Err(UserError::EmailTaken)
            }
            Err(sqlx::Error::Database(db)) if db.constraint() == Some("users_handle_uq") => {
                Err(UserError::HandleTaken)
            }
            Err(e) => Err(UserError::Database(e)),
        }
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<User>, UserError> {
        let sql = format!("SELECT {USER_SELECT} FROM users WHERE lower(email) = lower($1)");
        let row: Option<UserRow> = sqlx::query_as(&sql)
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(user_from_row))
    }

    async fn find_by_handle(&self, handle: &str) -> Result<Option<User>, UserError> {
        let sql =
            format!("SELECT {USER_SELECT} FROM users WHERE lower(claimed_handle) = lower($1)");
        let row: Option<UserRow> = sqlx::query_as(&sql)
            .bind(handle)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(user_from_row))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, UserError> {
        let sql = format!("SELECT {USER_SELECT} FROM users WHERE id = $1");
        let row: Option<UserRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(user_from_row))
    }

    async fn list_users(&self, filters: ListUsersFilters) -> Result<Vec<User>, UserError> {
        let limit = filters.limit.clamp(1, 200);
        let offset = filters.offset.max(0);
        let q_norm = filters
            .q
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let rows: Vec<UserRow> = if let Some(q) = q_norm.as_ref() {
            let pattern = format!("%{q}%");
            let sql = format!(
                "SELECT {USER_SELECT} FROM users
                 WHERE claimed_handle ILIKE $1 OR email ILIKE $1
                 ORDER BY created_at DESC
                 LIMIT $2 OFFSET $3"
            );
            sqlx::query_as(&sql)
                .bind(pattern)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        } else {
            let sql = format!(
                "SELECT {USER_SELECT} FROM users
                 ORDER BY created_at DESC
                 LIMIT $1 OFFSET $2"
            );
            sqlx::query_as(&sql)
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows.into_iter().map(user_from_row).collect())
    }

    async fn set_verification_token(
        &self,
        user_id: Uuid,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET email_verification_token = $2,
                   email_verification_expires_at = $3,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .bind(token)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_by_verification_token(&self, token: &str) -> Result<Option<User>, UserError> {
        // Pulls expiry inline so the handler can distinguish
        // "unknown token" (None) from "expired" (Some + now > expiry)
        // in a single round trip. The partial unique index on
        // `email_verification_token` makes this an indexed lookup.
        let sql = format!(
            "SELECT {USER_SELECT}, email_verification_expires_at \
             FROM users \
             WHERE email_verification_token = $1"
        );
        let row: Option<UserRowWithExpiry> = sqlx::query_as(&sql)
            .bind(token)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| {
            let (row, expires_at) = split_with_expiry(r);
            let mut u = user_from_row(row);
            u.email_verification_expires_at = expires_at;
            u
        }))
    }

    async fn mark_email_verified(&self, user_id: Uuid) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET email_verified_at = NOW(),
                   email_verification_token = NULL,
                   email_verification_expires_at = NULL,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_password(&self, user_id: Uuid, new_phc: &str) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET password_hash = $2,
                   password_changed_at = NOW(),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .bind(new_phc)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- Password reset ----------------------------------------------

    async fn set_password_reset_token(
        &self,
        user_id: Uuid,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET password_reset_token = $2,
                   password_reset_expires_at = $3,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .bind(token)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_by_password_reset_token(&self, token: &str) -> Result<Option<User>, UserError> {
        let sql = format!(
            "SELECT {USER_SELECT}, password_reset_expires_at \
             FROM users \
             WHERE password_reset_token = $1"
        );
        let row: Option<UserRowWithExpiry> = sqlx::query_as(&sql)
            .bind(token)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| {
            let (row, expires_at) = split_with_expiry(r);
            let mut u = user_from_row(row);
            u.password_reset_expires_at = expires_at;
            u
        }))
    }

    async fn complete_password_reset(&self, user_id: Uuid, new_phc: &str) -> Result<(), UserError> {
        // Single UPDATE so the password rotation, token wipe, and
        // password_changed_at bump are atomic. A concurrent reader
        // either sees the old hash + old changed_at (and may briefly
        // accept the old password) or the new hash + new changed_at —
        // never a torn intermediate state.
        sqlx::query(
            r#"
            UPDATE users
               SET password_hash = $2,
                   password_changed_at = NOW(),
                   password_reset_token = NULL,
                   password_reset_expires_at = NULL,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .bind(new_phc)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- Email change ------------------------------------------------

    async fn set_pending_email(
        &self,
        user_id: Uuid,
        new_email: &str,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET pending_email = lower($2),
                   pending_email_token = $3,
                   pending_email_expires_at = $4,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .bind(new_email)
        .bind(token)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_by_pending_email_token(&self, token: &str) -> Result<Option<User>, UserError> {
        let sql = format!(
            "SELECT {USER_SELECT}, pending_email_expires_at \
             FROM users \
             WHERE pending_email_token = $1"
        );
        let row: Option<UserRowWithExpiry> = sqlx::query_as(&sql)
            .bind(token)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| {
            let (row, expires_at) = split_with_expiry(r);
            let mut u = user_from_row(row);
            u.pending_email_expires_at = expires_at;
            u
        }))
    }

    async fn commit_pending_email(&self, user_id: Uuid) -> Result<(), UserError> {
        // Single UPDATE; the unique index on lower(email) catches the
        // race where another user signed up with the staged address
        // between set_pending_email and commit. Map that to
        // EmailTaken so the handler can surface a meaningful 409.
        let result = sqlx::query(
            r#"
            UPDATE users
               SET email = pending_email,
                   email_verified_at = NOW(),
                   pending_email = NULL,
                   pending_email_token = NULL,
                   pending_email_expires_at = NULL,
                   updated_at = NOW()
             WHERE id = $1 AND pending_email IS NOT NULL
            "#,
        )
        .bind(user_id)
        .execute(&self.pool)
        .await;
        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db)) if db.constraint() == Some("users_email_uq") => {
                Err(UserError::EmailTaken)
            }
            Err(e) => Err(UserError::Database(e)),
        }
    }

    // -- RSI handle verification ------------------------------------

    async fn set_rsi_verify_code(
        &self,
        user_id: Uuid,
        code: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET rsi_verify_code = $2,
                   rsi_verify_expires_at = $3,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .bind(code)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_rsi_verified(&self, user_id: Uuid) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET rsi_verified_at = NOW(),
                   rsi_verify_code = NULL,
                   rsi_verify_expires_at = NULL,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- TOTP 2FA ----------------------------------------------------

    async fn start_totp_setup(
        &self,
        user_id: Uuid,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET totp_secret_ciphertext = $2,
                   totp_secret_nonce = $3,
                   totp_setup_at = NOW(),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .bind(ciphertext)
        .bind(nonce)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_totp_secret(
        &self,
        user_id: Uuid,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, UserError> {
        let row: Option<(Option<Vec<u8>>, Option<Vec<u8>>)> = sqlx::query_as(
            "SELECT totp_secret_ciphertext, totp_secret_nonce FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(ct, n)| match (ct, n) {
            (Some(ct), Some(n)) => Some((ct, n)),
            _ => None,
        }))
    }

    async fn mark_totp_enabled(&self, user_id: Uuid) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET totp_enabled_at = NOW(),
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn disable_totp(&self, user_id: Uuid) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET totp_secret_ciphertext = NULL,
                   totp_secret_nonce = NULL,
                   totp_setup_at = NULL,
                   totp_enabled_at = NULL,
                   updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_shares_paused_until_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<DateTime<Utc>>, UserError> {
        let row: Option<(Option<DateTime<Utc>>,)> = sqlx::query_as(
            r#"
            SELECT shares_paused_until
              FROM users
             WHERE lower(claimed_handle) = lower($1)
            "#,
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(t,)| t))
    }

    async fn set_shares_paused_until_by_handle(
        &self,
        handle: &str,
        until: Option<DateTime<Utc>>,
    ) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET shares_paused_until = $2,
                   updated_at = NOW()
             WHERE lower(claimed_handle) = lower($1)
            "#,
        )
        .bind(handle)
        .bind(until)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_listing_opt_out_by_handle(&self, handle: &str) -> Result<bool, UserError> {
        // No-row collapses to `false` — same posture as the
        // `shares_paused_until` lookup above. The discover handler
        // walks a SpiceDB-supplied set, so any "user not found here"
        // is a stale handle we can safely treat as opt-in (the row
        // existence join below the SpiceDB call drops them anyway).
        let row: Option<(bool,)> = sqlx::query_as(
            r#"
            SELECT listing_opt_out
              FROM users
             WHERE lower(claimed_handle) = lower($1)
            "#,
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(v,)| v).unwrap_or(false))
    }

    async fn set_listing_opt_out_by_handle(
        &self,
        handle: &str,
        value: bool,
    ) -> Result<(), UserError> {
        sqlx::query(
            r#"
            UPDATE users
               SET listing_opt_out = $2,
                   updated_at = NOW()
             WHERE lower(claimed_handle) = lower($1)
            "#,
        )
        .bind(handle)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- Account deletion: pseudonymise events, then drop the user ----
    //
    // `events` rows are keyed by `claimed_handle`, not by `user_id`,
    // and there is no FK from `events` -> `users` (see migrations
    // 0001 and 0004). On `DELETE /v1/auth/me` we therefore can't rely
    // on a Postgres cascade. Instead we pseudonymise the deleted
    // user's events in-place: the row count + structural shape stays
    // intact (so a friend's shared timeline that joined through this
    // user's events doesn't suddenly develop holes), but the
    // identifying fields (`claimed_handle`, raw log line, parsed
    // payload) are replaced with a one-way tombstone.
    //
    // This satisfies Art. 17 (right to erasure) for the deleted user
    // while preserving aggregate integrity for *other* users whose
    // shared/org views may have aggregated against these rows.
    //
    // The companion FK chain handles the rest:
    //  * `devices.user_id` and `device_pairings.user_id` cascade.
    //  * SpiceDB relationships keyed on `user:<handle>` are NOT
    //    cleaned up here — that's a separate follow-up because the
    //    SpiceDB client lives outside the user-store boundary.
    async fn delete_user(&self, user_id: Uuid) -> Result<(), UserError> {
        let mut tx = self.pool.begin().await?;

        // Look up the handle inside the same tx so the pseudonymise
        // step can't race a concurrent profile rename. `Option` keeps
        // the call idempotent: deleting an already-gone user is a
        // no-op rather than an error.
        let handle: Option<String> =
            sqlx::query_scalar("SELECT claimed_handle FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;

        if let Some(h) = handle {
            // `deleted-<uuid>` is a non-resolvable tombstone — no
            // handle re-registration can reclaim it because the UUID
            // is fresh per deletion. Lower-case the original handle
            // on the lookup side: the events table accepts whatever
            // case the client sent, so a case-insensitive match
            // ensures we don't miss rows ingested with mixed casing.
            let pseudonym = format!("deleted-{}", Uuid::new_v4());
            sqlx::query(
                r#"
                UPDATE events
                SET claimed_handle = $1,
                    raw_line = '',
                    payload = '{}'::jsonb
                WHERE lower(claimed_handle) = lower($2)
                "#,
            )
            .bind(&pseudonym)
            .bind(&h)
            .execute(&mut *tx)
            .await?;

            // `contract_runs` materialises free-text log content
            // (contract name, step text) verbatim from the same source
            // the pseudonymisation above blanks in `events` — leaving it
            // in place would defeat the erasure this function exists to
            // do. `stat_rollup_state` doesn't hold free text, but its
            // `contracts_dirty = FALSE` after this handle's last read
            // would let a re-registered handle skip the rebuild and be
            // served the deleted user's rows straight from
            // `contract_runs` before their own tray ever uploads
            // anything. Dropping both here closes that window.
            //
            // Note: `session_summary`, `character_records`,
            // `stat_event_counts` and the rest of `stat_rollup_state`'s
            // dirty flags are equally unreachable from this function —
            // that gap predates this change and is a separate follow-up.
            sqlx::query("DELETE FROM contract_runs WHERE claimed_handle = lower($1)")
                .bind(&h)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM stat_rollup_state WHERE claimed_handle = lower($1)")
                .bind(&h)
                .execute(&mut *tx)
                .await?;
        }

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn purge_user(&self, user_id: Uuid) -> Result<(), UserError> {
        let mut tx = self.pool.begin().await?;

        // Handle resolved inside the tx, same reasoning as delete_user:
        // it cannot race a concurrent rename, and `Option` keeps the
        // call idempotent.
        let handle: Option<String> =
            sqlx::query_scalar("SELECT claimed_handle FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;

        if let Some(h) = handle {
            // events accepts whatever case the client sent, so match
            // case-insensitively or mixed-case rows survive the purge.
            sqlx::query("DELETE FROM events WHERE lower(claimed_handle) = lower($1)")
                .bind(&h)
                .execute(&mut *tx)
                .await?;

            // The derived tables. Every one of these is lower-cased at
            // write time (each carries a `*_lc_chk` CHECK), so bind the
            // lower-cased handle rather than matching case-insensitively.
            for table in [
                "contract_runs",
                "stat_rollup_state",
                "stat_event_counts",
                "session_summary",
                "character_records",
            ] {
                // Table names are a fixed literal list, never user
                // input -- format! here cannot interpolate anything a
                // caller controls. The HANDLE is still bound.
                sqlx::query(&format!(
                    "DELETE FROM {table} WHERE claimed_handle = lower($1)"
                ))
                .bind(&h)
                .execute(&mut *tx)
                .await?;
            }
        }

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
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
    pub struct MemoryUserStore {
        // Keyed by lowercase(email).
        rows: Mutex<HashMap<String, User>>,
        // Verification tokens. Public so the auth_routes test suite
        // can peek at freshly-issued tokens without re-implementing
        // the lookup — there's no public endpoint that returns a
        // verification token by design.
        pub tokens: Mutex<HashMap<String, (Uuid, DateTime<Utc>)>>,
        // Password-reset tokens. Same shape + same public-for-tests
        // rationale as `tokens`.
        pub reset_tokens: Mutex<HashMap<String, (Uuid, DateTime<Utc>)>>,
        // Pending-email tokens. Each entry pairs the token with both
        // the user id and the staged email so commit_pending_email
        // can swap addresses without an extra side-channel.
        pub pending_email_tokens: Mutex<HashMap<String, (Uuid, String, DateTime<Utc>)>>,
        // TOTP encrypted secret + nonce, keyed by user id. Lives off
        // the `User` struct in the in-memory impl too, mirroring the
        // Postgres `get_totp_secret` separate-query pattern.
        pub totp_secrets: Mutex<HashMap<Uuid, (Vec<u8>, Vec<u8>)>>,
        // Audit v2.1 §C — abuse-signal auto-pause. Keyed by
        // lower(claimed_handle). Lives off `User` for the same reason
        // as `totp_secrets`: the field is only ever read via a
        // dedicated query (`get_shares_paused_until_by_handle`), and
        // keeping it off the struct avoids touching every existing
        // SELECT in the Postgres impl.
        pub shares_paused: Mutex<HashMap<String, DateTime<Utc>>>,
        // Piece 4 — `/discover` listing opt-out. Keyed by
        // lower(claimed_handle). Absence = FALSE (opted in), matching
        // the Postgres column default. Same off-User rationale as
        // `shares_paused`: dedicated lookup + write methods, no
        // existing SELECT cares about this field.
        pub listing_opt_out: Mutex<HashMap<String, bool>>,
    }

    impl MemoryUserStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl UserStore for MemoryUserStore {
        async fn create(
            &self,
            email: &str,
            password_hash: &str,
            claimed_handle: &str,
        ) -> Result<User, UserError> {
            let mut rows = self.rows.lock().unwrap();
            let key = email.to_lowercase();
            if rows.contains_key(&key) {
                return Err(UserError::EmailTaken);
            }
            if rows
                .values()
                .any(|u| u.claimed_handle.eq_ignore_ascii_case(claimed_handle))
            {
                return Err(UserError::HandleTaken);
            }
            let user = User {
                id: Uuid::new_v4(),
                email: key.clone(),
                password_hash: password_hash.to_owned(),
                claimed_handle: claimed_handle.to_owned(),
                created_at: Utc::now(),
                email_verified_at: None,
                email_verification_expires_at: None,
                password_changed_at: Utc::now(),
                pending_email: None,
                password_reset_expires_at: None,
                pending_email_expires_at: None,
                rsi_verified_at: None,
                rsi_verify_code: None,
                rsi_verify_expires_at: None,
                totp_enabled_at: None,
            };
            rows.insert(key, user.clone());
            Ok(user)
        }

        async fn find_by_email(&self, email: &str) -> Result<Option<User>, UserError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows.get(&email.to_lowercase()).cloned())
        }

        async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, UserError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows.values().find(|u| u.id == id).cloned())
        }

        async fn find_by_handle(&self, handle: &str) -> Result<Option<User>, UserError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .values()
                .find(|u| u.claimed_handle.eq_ignore_ascii_case(handle))
                .cloned())
        }

        async fn list_users(&self, filters: ListUsersFilters) -> Result<Vec<User>, UserError> {
            let limit = filters.limit.clamp(1, 200) as usize;
            let offset = filters.offset.max(0) as usize;
            let q_lower = filters
                .q
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty());
            let rows = self.rows.lock().unwrap();
            let mut filtered: Vec<User> = rows
                .values()
                .filter(|u| match q_lower.as_ref() {
                    None => true,
                    Some(q) => {
                        u.claimed_handle.to_lowercase().contains(q)
                            || u.email.to_lowercase().contains(q)
                    }
                })
                .cloned()
                .collect();
            filtered.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(filtered.into_iter().skip(offset).take(limit).collect())
        }

        async fn set_verification_token(
            &self,
            user_id: Uuid,
            token: &str,
            expires_at: DateTime<Utc>,
        ) -> Result<(), UserError> {
            let mut tokens = self.tokens.lock().unwrap();
            // Drop any prior token for this user so the latest one wins
            // (mirrors the SQL UPDATE which overwrites the column).
            tokens.retain(|_, (uid, _)| *uid != user_id);
            tokens.insert(token.to_owned(), (user_id, expires_at));
            Ok(())
        }

        async fn find_by_verification_token(&self, token: &str) -> Result<Option<User>, UserError> {
            let tokens = self.tokens.lock().unwrap();
            let Some((user_id, expires_at)) = tokens.get(token).copied() else {
                return Ok(None);
            };
            drop(tokens);
            let rows = self.rows.lock().unwrap();
            let mut hit = rows.values().find(|u| u.id == user_id).cloned();
            if let Some(u) = hit.as_mut() {
                u.email_verification_expires_at = Some(expires_at);
            }
            Ok(hit)
        }

        async fn mark_email_verified(&self, user_id: Uuid) -> Result<(), UserError> {
            let mut tokens = self.tokens.lock().unwrap();
            tokens.retain(|_, (uid, _)| *uid != user_id);
            drop(tokens);
            let mut rows = self.rows.lock().unwrap();
            for u in rows.values_mut() {
                if u.id == user_id {
                    u.email_verified_at = Some(Utc::now());
                    u.email_verification_expires_at = None;
                }
            }
            Ok(())
        }

        async fn update_password(&self, user_id: Uuid, new_phc: &str) -> Result<(), UserError> {
            let mut rows = self.rows.lock().unwrap();
            for u in rows.values_mut() {
                if u.id == user_id {
                    u.password_hash = new_phc.to_owned();
                    u.password_changed_at = Utc::now();
                }
            }
            Ok(())
        }

        async fn delete_user(&self, user_id: Uuid) -> Result<(), UserError> {
            // Mirror the Postgres cascade: drop the user's row AND
            // every kind of outstanding token tied to the user.
            self.tokens
                .lock()
                .unwrap()
                .retain(|_, (uid, _)| *uid != user_id);
            self.reset_tokens
                .lock()
                .unwrap()
                .retain(|_, (uid, _)| *uid != user_id);
            self.pending_email_tokens
                .lock()
                .unwrap()
                .retain(|_, (uid, _, _)| *uid != user_id);
            self.rows.lock().unwrap().retain(|_, u| u.id != user_id);
            Ok(())
        }

        async fn purge_user(&self, user_id: Uuid) -> Result<(), UserError> {
            // The memory store holds no events or derived rollups, so
            // purge and delete are indistinguishable here. That is a
            // real limit of this double: the row-level difference
            // between the two flavours is only observable against
            // Postgres, and the route tests assert which STORE METHOD
            // was called rather than pretending to verify the SQL.
            self.delete_user(user_id).await
        }

        // -- Password reset --------------------------------------------

        async fn set_password_reset_token(
            &self,
            user_id: Uuid,
            token: &str,
            expires_at: DateTime<Utc>,
        ) -> Result<(), UserError> {
            let mut t = self.reset_tokens.lock().unwrap();
            t.retain(|_, (uid, _)| *uid != user_id);
            t.insert(token.to_owned(), (user_id, expires_at));
            Ok(())
        }

        async fn find_by_password_reset_token(
            &self,
            token: &str,
        ) -> Result<Option<User>, UserError> {
            let Some((user_id, expires_at)) = self.reset_tokens.lock().unwrap().get(token).copied()
            else {
                return Ok(None);
            };
            let mut hit = self
                .rows
                .lock()
                .unwrap()
                .values()
                .find(|u| u.id == user_id)
                .cloned();
            if let Some(u) = hit.as_mut() {
                u.password_reset_expires_at = Some(expires_at);
            }
            Ok(hit)
        }

        async fn complete_password_reset(
            &self,
            user_id: Uuid,
            new_phc: &str,
        ) -> Result<(), UserError> {
            self.reset_tokens
                .lock()
                .unwrap()
                .retain(|_, (uid, _)| *uid != user_id);
            let mut rows = self.rows.lock().unwrap();
            for u in rows.values_mut() {
                if u.id == user_id {
                    u.password_hash = new_phc.to_owned();
                    u.password_changed_at = Utc::now();
                }
            }
            Ok(())
        }

        // -- Email change ----------------------------------------------

        async fn set_pending_email(
            &self,
            user_id: Uuid,
            new_email: &str,
            token: &str,
            expires_at: DateTime<Utc>,
        ) -> Result<(), UserError> {
            let staged = new_email.to_lowercase();
            let mut t = self.pending_email_tokens.lock().unwrap();
            t.retain(|_, (uid, _, _)| *uid != user_id);
            t.insert(token.to_owned(), (user_id, staged.clone(), expires_at));
            drop(t);
            let mut rows = self.rows.lock().unwrap();
            for u in rows.values_mut() {
                if u.id == user_id {
                    u.pending_email = Some(staged.clone());
                }
            }
            Ok(())
        }

        async fn find_by_pending_email_token(
            &self,
            token: &str,
        ) -> Result<Option<User>, UserError> {
            let Some((user_id, _, expires_at)) = self
                .pending_email_tokens
                .lock()
                .unwrap()
                .get(token)
                .cloned()
            else {
                return Ok(None);
            };
            let mut hit = self
                .rows
                .lock()
                .unwrap()
                .values()
                .find(|u| u.id == user_id)
                .cloned();
            if let Some(u) = hit.as_mut() {
                u.pending_email_expires_at = Some(expires_at);
            }
            Ok(hit)
        }

        async fn start_totp_setup(
            &self,
            user_id: Uuid,
            ciphertext: &[u8],
            nonce: &[u8],
        ) -> Result<(), UserError> {
            self.totp_secrets
                .lock()
                .unwrap()
                .insert(user_id, (ciphertext.to_vec(), nonce.to_vec()));
            Ok(())
        }

        async fn get_totp_secret(
            &self,
            user_id: Uuid,
        ) -> Result<Option<(Vec<u8>, Vec<u8>)>, UserError> {
            Ok(self.totp_secrets.lock().unwrap().get(&user_id).cloned())
        }

        async fn mark_totp_enabled(&self, user_id: Uuid) -> Result<(), UserError> {
            let mut rows = self.rows.lock().unwrap();
            for u in rows.values_mut() {
                if u.id == user_id {
                    u.totp_enabled_at = Some(Utc::now());
                }
            }
            Ok(())
        }

        async fn disable_totp(&self, user_id: Uuid) -> Result<(), UserError> {
            self.totp_secrets.lock().unwrap().remove(&user_id);
            let mut rows = self.rows.lock().unwrap();
            for u in rows.values_mut() {
                if u.id == user_id {
                    u.totp_enabled_at = None;
                }
            }
            Ok(())
        }

        async fn set_rsi_verify_code(
            &self,
            user_id: Uuid,
            code: &str,
            expires_at: DateTime<Utc>,
        ) -> Result<(), UserError> {
            let mut rows = self.rows.lock().unwrap();
            for u in rows.values_mut() {
                if u.id == user_id {
                    u.rsi_verify_code = Some(code.to_owned());
                    u.rsi_verify_expires_at = Some(expires_at);
                }
            }
            Ok(())
        }

        async fn mark_rsi_verified(&self, user_id: Uuid) -> Result<(), UserError> {
            let mut rows = self.rows.lock().unwrap();
            for u in rows.values_mut() {
                if u.id == user_id {
                    u.rsi_verified_at = Some(Utc::now());
                    u.rsi_verify_code = None;
                    u.rsi_verify_expires_at = None;
                }
            }
            Ok(())
        }

        async fn commit_pending_email(&self, user_id: Uuid) -> Result<(), UserError> {
            // Pull the staged email out of the token map so the swap
            // is the source of truth, then drop tokens for this user.
            let staged = {
                let mut t = self.pending_email_tokens.lock().unwrap();
                let key = t
                    .iter()
                    .find_map(|(k, (uid, _, _))| (*uid == user_id).then(|| k.clone()));
                key.and_then(|k| t.remove(&k).map(|(_, email, _)| email))
            };
            let Some(staged) = staged else {
                return Ok(());
            };
            let mut rows = self.rows.lock().unwrap();
            // Re-key the map: the entry was indexed by the old email.
            let old_key = rows
                .iter()
                .find_map(|(k, u)| (u.id == user_id).then(|| k.clone()));
            // Conflict check: another user may have grabbed the staged
            // address between set and commit.
            if rows.contains_key(&staged) && old_key.as_deref() != Some(&staged) {
                return Err(UserError::EmailTaken);
            }
            if let Some(old) = old_key {
                if let Some(mut u) = rows.remove(&old) {
                    u.email = staged.clone();
                    u.email_verified_at = Some(Utc::now());
                    u.pending_email = None;
                    u.pending_email_expires_at = None;
                    rows.insert(staged, u);
                }
            }
            Ok(())
        }

        async fn get_shares_paused_until_by_handle(
            &self,
            handle: &str,
        ) -> Result<Option<DateTime<Utc>>, UserError> {
            let key = handle.to_ascii_lowercase();
            Ok(self.shares_paused.lock().unwrap().get(&key).copied())
        }

        async fn set_shares_paused_until_by_handle(
            &self,
            handle: &str,
            until: Option<DateTime<Utc>>,
        ) -> Result<(), UserError> {
            let key = handle.to_ascii_lowercase();
            let mut map = self.shares_paused.lock().unwrap();
            match until {
                Some(t) => {
                    map.insert(key, t);
                }
                None => {
                    map.remove(&key);
                }
            }
            Ok(())
        }

        async fn get_listing_opt_out_by_handle(&self, handle: &str) -> Result<bool, UserError> {
            let key = handle.to_ascii_lowercase();
            Ok(self
                .listing_opt_out
                .lock()
                .unwrap()
                .get(&key)
                .copied()
                .unwrap_or(false))
        }

        async fn set_listing_opt_out_by_handle(
            &self,
            handle: &str,
            value: bool,
        ) -> Result<(), UserError> {
            let key = handle.to_ascii_lowercase();
            let mut map = self.listing_opt_out.lock().unwrap();
            // Drop the entry on FALSE so the map keeps only the small
            // set of opted-out handles; mirrors the lookup default.
            if value {
                map.insert(key, true);
            } else {
                map.remove(&key);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryUserStore;
    use super::*;

    #[test]
    fn hash_then_verify_round_trips() {
        let phc = hash_password("hunter2-with-some-padding").unwrap();
        assert!(verify_password("hunter2-with-some-padding", &phc));
        assert!(!verify_password("wrong-password", &phc));
    }

    #[test]
    fn verify_rejects_garbage_phc() {
        // Not a valid PHC string at all — must not panic, must return false.
        assert!(!verify_password("anything", "not-a-real-phc-string"));
    }

    #[test]
    fn validate_handle_accepts_well_formed() {
        assert!(validate_handle("TheCodeSaiyan"));
        assert!(validate_handle("daisy_2025"));
        assert!(validate_handle("a-b_c"));
        assert!(validate_handle("X"));
        assert!(validate_handle(&"a".repeat(64)));
    }

    #[test]
    fn validate_handle_rejects_malformed() {
        assert!(!validate_handle(""));
        assert!(!validate_handle(&"a".repeat(65)));
        assert!(!validate_handle("with space"));
        assert!(!validate_handle("emoji😀"));
        assert!(!validate_handle("dot.notation"));
        assert!(!validate_handle("<script>"));
        assert!(!validate_handle("../path"));
    }

    #[tokio::test]
    async fn memory_store_rejects_duplicate_email() {
        let store = MemoryUserStore::new();
        let phc = hash_password("password-123-abc").unwrap();
        store
            .create("daisy@example.com", &phc, "TheCodeSaiyan")
            .await
            .unwrap();
        let err = store
            .create("DAISY@example.com", &phc, "OtherHandle")
            .await
            .unwrap_err();
        assert!(matches!(err, UserError::EmailTaken));
    }

    #[tokio::test]
    async fn memory_store_rejects_duplicate_handle() {
        let store = MemoryUserStore::new();
        let phc = hash_password("password-123-abc").unwrap();
        store
            .create("a@example.com", &phc, "TheCodeSaiyan")
            .await
            .unwrap();
        let err = store
            .create("b@example.com", &phc, "thecodesaiyan")
            .await
            .unwrap_err();
        assert!(matches!(err, UserError::HandleTaken));
    }

    #[tokio::test]
    async fn memory_store_finds_by_email_case_insensitive() {
        let store = MemoryUserStore::new();
        let phc = hash_password("password-123-abc").unwrap();
        store
            .create("daisy@example.com", &phc, "TheCodeSaiyan")
            .await
            .unwrap();
        assert!(store
            .find_by_email("DAISY@example.com")
            .await
            .unwrap()
            .is_some());
        assert!(store
            .find_by_email("nobody@example.com")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn memory_store_update_password_replaces_hash() {
        let store = MemoryUserStore::new();
        let phc = hash_password("password-123-abc").unwrap();
        let user = store
            .create("daisy@example.com", &phc, "TheCodeSaiyan")
            .await
            .unwrap();
        let new_phc = hash_password("brand-new-password-456").unwrap();
        store.update_password(user.id, &new_phc).await.unwrap();

        let found = store.find_by_id(user.id).await.unwrap().unwrap();
        assert_eq!(found.password_hash, new_phc);
        assert!(verify_password(
            "brand-new-password-456",
            &found.password_hash
        ));
        assert!(!verify_password("password-123-abc", &found.password_hash));
    }

    #[tokio::test]
    async fn memory_store_listing_opt_out_defaults_false_and_round_trips() {
        // Piece 4 — discover listing opt-out. The default is "appear
        // in the listing" (false); the setter toggles and survives a
        // read-back; case-insensitive on the handle key.
        let store = MemoryUserStore::new();
        assert!(!store.get_listing_opt_out_by_handle("Alice").await.unwrap());
        store
            .set_listing_opt_out_by_handle("Alice", true)
            .await
            .unwrap();
        assert!(store.get_listing_opt_out_by_handle("alice").await.unwrap());
        store
            .set_listing_opt_out_by_handle("ALICE", false)
            .await
            .unwrap();
        assert!(!store.get_listing_opt_out_by_handle("Alice").await.unwrap());
    }

    #[tokio::test]
    async fn memory_store_delete_user_removes_row_and_tokens() {
        let store = MemoryUserStore::new();
        let phc = hash_password("password-123-abc").unwrap();
        let user = store
            .create("daisy@example.com", &phc, "TheCodeSaiyan")
            .await
            .unwrap();
        store
            .set_verification_token(user.id, "tok-xyz", Utc::now() + chrono::Duration::hours(1))
            .await
            .unwrap();

        store.delete_user(user.id).await.unwrap();

        assert!(store.find_by_id(user.id).await.unwrap().is_none());
        assert!(store
            .find_by_email("daisy@example.com")
            .await
            .unwrap()
            .is_none());
        assert!(store
            .find_by_verification_token("tok-xyz")
            .await
            .unwrap()
            .is_none());
    }

    // Postgres-gated round-trip (env-gated integration test, same pattern
    // as `contract_runs_postgres_round_trip` in repo.rs). Runs ONLY when
    // STARSTATS_TEST_DATABASE_URL points at a real Postgres; offline
    // `cargo test` skips it. `MemoryUserStore::delete_user` above can't
    // reach this -- it's a plain in-memory HashMap with no concept of
    // `contract_runs` or `stat_rollup_state` -- so this is the only test
    // exercising `PostgresUserStore::delete_user`'s cleanup of those two
    // derived tables. Without it, a deleted user's contract history
    // (free-text contract names + step text, lifted verbatim from their
    // log) would survive account deletion, and a handle re-registration
    // could read a stale `contracts_dirty = FALSE` and be served the
    // previous owner's rows before their own tray ever uploads anything.
    #[tokio::test]
    async fn postgres_delete_user_clears_contract_runs_and_rollup_state() {
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

        let handle = "deleteuser_rollup_probe";
        let email = "deleteuser_rollup_probe@example.com";
        sqlx::query("DELETE FROM users WHERE lower(claimed_handle) = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean probe users");
        sqlx::query("DELETE FROM contract_runs WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean probe contract_runs");
        sqlx::query("DELETE FROM stat_rollup_state WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean probe rollup state");

        let store = PostgresUserStore::new(pool.clone());
        let phc = hash_password("password-123-abc").unwrap();
        let user = store
            .create(email, &phc, handle)
            .await
            .expect("create probe user");

        sqlx::query(
            "INSERT INTO contract_runs (claimed_handle, mission_id, name, state, closed_by)
             VALUES ($1, 'deleteuser-probe-mission', 'Probe Contract', 'completed', 'hud_complete')",
        )
        .bind(handle)
        .execute(&pool)
        .await
        .expect("seed probe contract_runs row");
        sqlx::query(
            "INSERT INTO stat_rollup_state (claimed_handle, sessions_dirty, contracts_dirty, updated_at)
             VALUES ($1, FALSE, FALSE, now())",
        )
        .bind(handle)
        .execute(&pool)
        .await
        .expect("seed probe stat_rollup_state row");

        store
            .delete_user(user.id)
            .await
            .expect("delete_user must not error");

        let remaining_runs: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM contract_runs WHERE claimed_handle = $1")
                .bind(handle)
                .fetch_one(&pool)
                .await
                .expect("count contract_runs");
        assert_eq!(
            remaining_runs, 0,
            "contract_runs must not survive account deletion — it carries verbatim log text"
        );

        let remaining_rollup: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM stat_rollup_state WHERE claimed_handle = $1")
                .bind(handle)
                .fetch_one(&pool)
                .await
                .expect("count stat_rollup_state");
        assert_eq!(
            remaining_rollup, 0,
            "stale contracts_dirty = FALSE must not survive to be inherited by a re-registered handle"
        );

        assert!(store.find_by_id(user.id).await.unwrap().is_none());
    }
}
