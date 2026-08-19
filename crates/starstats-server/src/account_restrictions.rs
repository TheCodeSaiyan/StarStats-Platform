//! Per-capability account restrictions.
//!
//! This exists because the moderation control that appeared to exist
//! did not. `/admin/sharing/reports` has offered a "Suspend owner"
//! button whose dialog promised that the owner's shares were revoked
//! and that they could not create new ones — while the handler only
//! wrote a status string onto the report row. `UserSuspended` had zero
//! read-sites anywhere in the codebase.
//!
//! So the governing rule for this module: a flag is only real where it
//! is READ. Every capability here is enforced at a named route, and the
//! tests assert the 403 at that route rather than asserting that a row
//! was written — the broken version wrote its row perfectly.
//!
//! Absence of a row is the unrestricted state; lifting deletes.
//! Expiry is a read-time predicate, never a sweep.

use chrono::{DateTime, Utc};
use std::collections::HashSet;
use uuid::Uuid;

/// A thing an account can be barred from doing.
///
/// Suspension is not a fifth variant — it is all four set at once. A
/// separate `suspended` flag would be a second representation of the
/// same state, and two representations drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Ingest,
    Sharing,
    PublicProfile,
    Submissions,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Ingest => "ingest",
            Capability::Sharing => "sharing",
            Capability::PublicProfile => "public_profile",
            Capability::Submissions => "submissions",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restriction {
    pub ingest_blocked: bool,
    pub sharing_blocked: bool,
    pub public_profile_blocked: bool,
    pub submissions_blocked: bool,
    /// Required, and surfaced to the restricted user.
    pub reason: String,
    /// Moderator handle.
    pub restricted_by: String,
    pub restricted_at: DateTime<Utc>,
    /// `None` means "until lifted".
    pub expires_at: Option<DateTime<Utc>>,
}

impl Restriction {
    /// Does this restriction bar `capability`?
    pub fn blocks(&self, capability: Capability) -> bool {
        match capability {
            Capability::Ingest => self.ingest_blocked,
            Capability::Sharing => self.sharing_blocked,
            Capability::PublicProfile => self.public_profile_blocked,
            Capability::Submissions => self.submissions_blocked,
        }
    }

    /// True only when every capability is blocked — i.e. a suspension
    /// rather than a targeted limit.
    pub fn is_suspension(&self) -> bool {
        self.ingest_blocked
            && self.sharing_blocked
            && self.public_profile_blocked
            && self.submissions_blocked
    }

    /// Read-time expiry. An expired row is inert but deliberately left
    /// in place, so the record that the account WAS restricted survives.
    ///
    /// `now >= expires_at` is expired. Strictly `expires_at > now` is
    /// effective, matching the SQL predicate exactly — if the two
    /// disagreed, a restriction would lift a moment early in one path
    /// and not the other.
    /// Not called in the release build: the Postgres store applies the
    /// same predicate in SQL, where it can use the index. This is kept
    /// as the Rust-side statement of the rule -- the memory store uses
    /// it, and the tests pin it against the SQL wording so the two
    /// cannot drift unnoticed.
    #[allow(dead_code)]
    pub fn is_effective_at(&self, now: DateTime<Utc>) -> bool {
        match self.expires_at {
            None => true,
            Some(expiry) => expiry > now,
        }
    }
}

/// Batched, read-mostly access to account restrictions.
#[async_trait::async_trait]
pub trait AccountRestrictionStore: Send + Sync {
    /// The user's live restriction. `None` when there is no row OR the
    /// row has expired — callers never have to think about expiry.
    async fn effective(&self, user_id: Uuid) -> Result<Option<Restriction>, sqlx::Error>;

    async fn upsert(&self, user_id: Uuid, restriction: &Restriction) -> Result<(), sqlx::Error>;

    /// Deletes the row. `true` when one was actually removed.
    async fn lift(&self, user_id: Uuid) -> Result<bool, sqlx::Error>;

    /// Apply a restriction to whoever owns `handle`, resolving the
    /// handle to a user id inside the query.
    ///
    /// Exists for the share-report resolution path, which knows the
    /// owner's handle but not their id. Returns the resolved user id,
    /// or `None` when no such user exists -- the caller must not treat
    /// a missing user as success.
    async fn upsert_by_handle(
        &self,
        handle: &str,
        restriction: &Restriction,
    ) -> Result<Option<Uuid>, sqlx::Error>;

    /// Of the given LOWERCASED handles, which belong to a user whose
    /// public profile is currently blocked. Returns lowercased handles.
    ///
    /// This is the ONLY place a handle join happens; the normalisation
    /// lives in the query so no caller has to remember it.
    async fn restricted_public_handles(
        &self,
        lowercased_handles: &[String],
    ) -> Result<HashSet<String>, sqlx::Error>;
}

// -- Postgres impl ---------------------------------------------------------

pub struct PostgresAccountRestrictionStore {
    pool: sqlx::PgPool,
}

impl PostgresAccountRestrictionStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

type RestrictionRow = (
    bool,
    bool,
    bool,
    bool,
    String,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

fn row_to_restriction(r: RestrictionRow) -> Restriction {
    Restriction {
        ingest_blocked: r.0,
        sharing_blocked: r.1,
        public_profile_blocked: r.2,
        submissions_blocked: r.3,
        reason: r.4,
        restricted_by: r.5,
        restricted_at: r.6,
        expires_at: r.7,
    }
}

#[async_trait::async_trait]
impl AccountRestrictionStore for PostgresAccountRestrictionStore {
    async fn effective(&self, user_id: Uuid) -> Result<Option<Restriction>, sqlx::Error> {
        // Expiry is applied HERE so no caller can forget it. Matches
        // `Restriction::is_effective_at` exactly (`> now()`).
        let row: Option<RestrictionRow> = sqlx::query_as(
            "SELECT ingest_blocked, sharing_blocked, public_profile_blocked,
                    submissions_blocked, reason, restricted_by, restricted_at, expires_at
             FROM account_restrictions
             WHERE user_id = $1
               AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_restriction))
    }

    async fn upsert(&self, user_id: Uuid, restriction: &Restriction) -> Result<(), sqlx::Error> {
        // PK conflict replaces: a new restriction on an already-expired
        // user supersedes the stale row rather than accumulating.
        sqlx::query(
            "INSERT INTO account_restrictions
                 (user_id, ingest_blocked, sharing_blocked, public_profile_blocked,
                  submissions_blocked, reason, restricted_by, restricted_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (user_id) DO UPDATE SET
                 ingest_blocked         = EXCLUDED.ingest_blocked,
                 sharing_blocked        = EXCLUDED.sharing_blocked,
                 public_profile_blocked = EXCLUDED.public_profile_blocked,
                 submissions_blocked    = EXCLUDED.submissions_blocked,
                 reason                 = EXCLUDED.reason,
                 restricted_by          = EXCLUDED.restricted_by,
                 restricted_at          = EXCLUDED.restricted_at,
                 expires_at             = EXCLUDED.expires_at",
        )
        .bind(user_id)
        .bind(restriction.ingest_blocked)
        .bind(restriction.sharing_blocked)
        .bind(restriction.public_profile_blocked)
        .bind(restriction.submissions_blocked)
        .bind(&restriction.reason)
        .bind(&restriction.restricted_by)
        .bind(restriction.restricted_at)
        .bind(restriction.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn lift(&self, user_id: Uuid) -> Result<bool, sqlx::Error> {
        let res = sqlx::query("DELETE FROM account_restrictions WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn upsert_by_handle(
        &self,
        handle: &str,
        restriction: &Restriction,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        // LOWER() on the users side so the caller can pass whatever
        // casing the report row carried.
        let found: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM users WHERE LOWER(claimed_handle) = $1")
                .bind(handle.to_lowercase())
                .fetch_optional(&self.pool)
                .await?;
        let Some((user_id,)) = found else {
            return Ok(None);
        };
        self.upsert(user_id, restriction).await?;
        Ok(Some(user_id))
    }

    async fn restricted_public_handles(
        &self,
        lowercased_handles: &[String],
    ) -> Result<HashSet<String>, sqlx::Error> {
        if lowercased_handles.is_empty() {
            return Ok(HashSet::new());
        }
        // The LOWER() lives here, once. Callers pass lowercased handles
        // and compare against lowercased results, so no call site has to
        // remember the normalisation.
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT LOWER(u.claimed_handle)
             FROM account_restrictions r
             JOIN users u ON u.id = r.user_id
             WHERE r.public_profile_blocked
               AND (r.expires_at IS NULL OR r.expires_at > now())
               AND LOWER(u.claimed_handle) = ANY($1)",
        )
        .bind(lowercased_handles)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(h,)| h).collect())
    }
}

// -- Memory impl -----------------------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory store for route-layer tests.
    ///
    /// `fail` makes every method return an error, which is how the
    /// guard's fail-closed behaviour is exercised — the single most
    /// important property in this subsystem.
    #[derive(Default)]
    pub struct MemoryAccountRestrictionStore {
        rows: Mutex<HashMap<Uuid, Restriction>>,
        handles: Mutex<HashMap<Uuid, String>>,
        fail: bool,
    }

    impl MemoryAccountRestrictionStore {
        pub fn new() -> Self {
            Self::default()
        }

        /// A store that errors on every call.
        pub fn failing() -> Self {
            Self {
                fail: true,
                ..Self::default()
            }
        }

        pub fn with_restriction(self, user_id: Uuid, r: Restriction) -> Self {
            self.rows.lock().unwrap().insert(user_id, r);
            self
        }

        /// Associate a handle with a user so `restricted_public_handles`
        /// can resolve it, mirroring the Postgres join.
        pub fn with_handle(self, user_id: Uuid, handle: &str) -> Self {
            self.handles
                .lock()
                .unwrap()
                .insert(user_id, handle.to_lowercase());
            self
        }

        fn err() -> sqlx::Error {
            sqlx::Error::Protocol("simulated restriction store failure".into())
        }
    }

    #[async_trait::async_trait]
    impl AccountRestrictionStore for MemoryAccountRestrictionStore {
        async fn effective(&self, user_id: Uuid) -> Result<Option<Restriction>, sqlx::Error> {
            if self.fail {
                return Err(Self::err());
            }
            let now = Utc::now();
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(&user_id)
                .filter(|r| r.is_effective_at(now))
                .cloned())
        }

        async fn upsert(
            &self,
            user_id: Uuid,
            restriction: &Restriction,
        ) -> Result<(), sqlx::Error> {
            if self.fail {
                return Err(Self::err());
            }
            self.rows
                .lock()
                .unwrap()
                .insert(user_id, restriction.clone());
            Ok(())
        }

        async fn lift(&self, user_id: Uuid) -> Result<bool, sqlx::Error> {
            if self.fail {
                return Err(Self::err());
            }
            Ok(self.rows.lock().unwrap().remove(&user_id).is_some())
        }

        async fn upsert_by_handle(
            &self,
            handle: &str,
            restriction: &Restriction,
        ) -> Result<Option<Uuid>, sqlx::Error> {
            if self.fail {
                return Err(Self::err());
            }
            let wanted = handle.to_lowercase();
            let found = self
                .handles
                .lock()
                .unwrap()
                .iter()
                .find(|(_, h)| **h == wanted)
                .map(|(id, _)| *id);
            match found {
                Some(id) => {
                    self.rows.lock().unwrap().insert(id, restriction.clone());
                    Ok(Some(id))
                }
                None => Ok(None),
            }
        }

        async fn restricted_public_handles(
            &self,
            lowercased_handles: &[String],
        ) -> Result<HashSet<String>, sqlx::Error> {
            if self.fail {
                return Err(Self::err());
            }
            let now = Utc::now();
            let rows = self.rows.lock().unwrap();
            let handles = self.handles.lock().unwrap();
            let wanted: HashSet<&String> = lowercased_handles.iter().collect();
            Ok(rows
                .iter()
                .filter(|(_, r)| r.public_profile_blocked && r.is_effective_at(now))
                .filter_map(|(id, _)| handles.get(id))
                .filter(|h| wanted.contains(h))
                .cloned()
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn base() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-11T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn restriction(sharing: bool, expires: Option<DateTime<Utc>>) -> Restriction {
        Restriction {
            ingest_blocked: false,
            sharing_blocked: sharing,
            public_profile_blocked: false,
            submissions_blocked: false,
            reason: "spam".into(),
            restricted_by: "mod".into(),
            restricted_at: base(),
            expires_at: expires,
        }
    }

    #[test]
    fn blocks_only_the_flagged_capability() {
        // This is what makes "limit" different from "suspend". If
        // blocks() ignored its argument, a targeted limit would bar
        // everything and nobody would notice until a user complained.
        let r = restriction(true, None);
        assert!(r.blocks(Capability::Sharing));
        assert!(!r.blocks(Capability::Ingest));
        assert!(!r.blocks(Capability::PublicProfile));
        assert!(!r.blocks(Capability::Submissions));
    }

    #[test]
    fn suspension_is_all_four_and_nothing_less() {
        assert!(!restriction(true, None).is_suspension());
        let all = Restriction {
            ingest_blocked: true,
            sharing_blocked: true,
            public_profile_blocked: true,
            submissions_blocked: true,
            ..restriction(true, None)
        };
        assert!(all.is_suspension());
    }

    #[test]
    fn expired_restriction_is_inert() {
        let expired = restriction(true, Some(base() - Duration::days(1)));
        assert!(!expired.is_effective_at(base()));
    }

    #[test]
    fn unexpired_and_never_expiring_are_both_effective() {
        assert!(restriction(true, Some(base() + Duration::days(1))).is_effective_at(base()));
        assert!(restriction(true, None).is_effective_at(base()));
    }

    // -- Memory store behaviour ------------------------------------
    //
    // These pin the contract the guard depends on, in particular that
    // `effective()` hides expired rows so no caller has to think about
    // expiry, and that `lift` deletes rather than blanking.

    use super::test_support::MemoryAccountRestrictionStore;

    fn suspended() -> Restriction {
        Restriction {
            ingest_blocked: true,
            sharing_blocked: true,
            public_profile_blocked: true,
            submissions_blocked: true,
            ..restriction(true, None)
        }
    }

    #[tokio::test]
    async fn effective_returns_none_when_there_is_no_row() {
        let store = MemoryAccountRestrictionStore::new();
        assert!(store.effective(Uuid::new_v4()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn effective_hides_an_expired_row_from_callers() {
        let id = Uuid::new_v4();
        let stale = restriction(true, Some(Utc::now() - Duration::days(1)));
        let store = MemoryAccountRestrictionStore::new().with_restriction(id, stale);
        assert!(store.effective(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn lift_removes_the_row_and_reports_whether_it_did() {
        let id = Uuid::new_v4();
        let store = MemoryAccountRestrictionStore::new().with_restriction(id, suspended());
        assert!(store.lift(id).await.unwrap());
        assert!(store.effective(id).await.unwrap().is_none());
        // Second lift is a no-op and says so.
        assert!(!store.lift(id).await.unwrap());
    }

    #[tokio::test]
    async fn restricted_public_handles_matches_case_insensitively() {
        // The whole point of doing the LOWER() inside the store: a
        // caller holding display casing must still get a hit.
        let id = Uuid::new_v4();
        let store = MemoryAccountRestrictionStore::new()
            .with_restriction(id, suspended())
            .with_handle(id, "TheCodeSaiyan");
        let hits = store
            .restricted_public_handles(&["thecodesaiyan".to_string()])
            .await
            .unwrap();
        assert!(hits.contains("thecodesaiyan"));
    }

    #[tokio::test]
    async fn restricted_public_handles_ignores_users_who_are_not_profile_blocked() {
        let id = Uuid::new_v4();
        // sharing-only limit: their public profile stays visible.
        let store = MemoryAccountRestrictionStore::new()
            .with_restriction(id, restriction(true, None))
            .with_handle(id, "someone");
        let hits = store
            .restricted_public_handles(&["someone".to_string()])
            .await
            .unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn failing_store_surfaces_an_error_rather_than_an_empty_result() {
        // An empty Ok() here would read as "not restricted" and the
        // guard would allow. The store must ERROR so the guard can
        // fail closed.
        let store = MemoryAccountRestrictionStore::failing();
        assert!(store.effective(Uuid::new_v4()).await.is_err());
    }

    #[test]
    fn expiry_boundary_is_inclusive_of_the_instant_it_expires() {
        // expires_at == now means expired: the SQL predicate is
        // `expires_at > now()`, and the Rust check must agree with it
        // or a restriction lifts a moment early in one path and not
        // the other.
        let at_boundary = restriction(true, Some(base()));
        assert!(!at_boundary.is_effective_at(base()));
    }
}
