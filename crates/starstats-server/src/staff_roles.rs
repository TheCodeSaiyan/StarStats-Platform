//! Site-wide staff role grants.
//!
//! Two roles today: `moderator` (submission moderation only) and
//! `admin` (everything moderators can do, plus the rest of the admin
//! surface). Stored in the `staff_roles` table -- see migration
//! `0019_staff_roles.sql` for the schema. Distinct from SpiceDB
//! org-level relations.
//!
//! The auth extractor consults this store when the request hits an
//! admin route; routes that don't touch admin functionality skip the
//! lookup entirely (see `admin_routes::RequireAdmin`).
//!
//! Bootstrap path: on every startup, `bootstrap_admins_from_env`
//! reads `STARSTATS_BOOTSTRAP_ADMIN_HANDLES` (comma-separated handles)
//! and grants `admin` idempotently. Each successful grant writes one
//! `audit_log` row with `actor_sub = NULL` (system action).

use crate::audit::{AuditEntry, AuditError, AuditLog};
use crate::users::{User, UserError, UserStore};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashSet;
use std::str::FromStr;
use uuid::Uuid;

/// Site-wide staff role. Order matters: `Admin` implies every
/// `Moderator` permission via `StaffRoleSet::has`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StaffRole {
    Moderator,
    Admin,
}

impl StaffRole {
    pub fn as_str(self) -> &'static str {
        match self {
            StaffRole::Moderator => "moderator",
            StaffRole::Admin => "admin",
        }
    }
}

impl FromStr for StaffRole {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "moderator" => Ok(StaffRole::Moderator),
            "admin" => Ok(StaffRole::Admin),
            other => Err(format!("unknown staff role: {other}")),
        }
    }
}

/// Set of active staff grants for a single user. `Admin` implies
/// `Moderator` even when the moderator row is absent -- handlers
/// should call `has(StaffRole::Moderator)` rather than
/// `contains(&StaffRole::Moderator)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StaffRoleSet(HashSet<StaffRole>);

impl StaffRoleSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_iter<I: IntoIterator<Item = StaffRole>>(roles: I) -> Self {
        Self(roles.into_iter().collect())
    }

    pub fn has(&self, role: StaffRole) -> bool {
        match role {
            StaffRole::Admin => self.0.contains(&StaffRole::Admin),
            // Admins inherit moderator privileges.
            StaffRole::Moderator => {
                self.0.contains(&StaffRole::Moderator) || self.0.contains(&StaffRole::Admin)
            }
        }
    }

    pub fn as_strings(&self) -> Vec<String> {
        let mut v: Vec<String> = self.0.iter().map(|r| r.as_str().to_owned()).collect();
        v.sort();
        v
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StaffRoleError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("user error: {0}")]
    User(#[from] UserError),
    #[error("audit log error: {0}")]
    Audit(#[from] AuditError),
}

/// One row from `staff_roles`, post-projection. Used by the admin
/// staff-list endpoint (later slice); kept here so the trait is
/// closed.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Surfaced by the admin staff-list endpoint in a later slice.
pub struct StaffRoleGrant {
    pub id: Uuid,
    pub user_id: Uuid,
    pub role: StaffRole,
    pub granted_at: DateTime<Utc>,
    pub granted_by_user_id: Option<Uuid>,
    pub reason: Option<String>,
}

#[async_trait]
pub trait StaffRoleStore: Send + Sync + 'static {
    /// Active roles for a single user. Used by the admin auth
    /// extractor on every admin request -- keep it tight.
    async fn list_active_for_user(&self, user_id: Uuid) -> Result<StaffRoleSet, StaffRoleError>;

    /// Grant a role idempotently. Returns `true` if a new active grant
    /// was created, `false` if the user already had an active grant
    /// for that role.
    ///
    /// `granted_by` is `None` for system actions (env-var bootstrap)
    /// and `Some(admin_user_id)` for UI-driven grants.
    async fn grant(
        &self,
        user_id: Uuid,
        role: StaffRole,
        granted_by: Option<Uuid>,
        reason: Option<&str>,
    ) -> Result<bool, StaffRoleError>;

    /// Soft-revoke the active grant for (user, role). Returns `true`
    /// if an active grant was found and stamped, `false` if no
    /// active grant existed (idempotent). The historical row is
    /// preserved with `revoked_at` + `revoked_by_user_id` so the
    /// chain "who used to be a moderator" remains queryable.
    async fn revoke(
        &self,
        user_id: Uuid,
        role: StaffRole,
        revoked_by: Option<Uuid>,
    ) -> Result<bool, StaffRoleError>;
}

pub struct PostgresStaffRoleStore {
    pool: PgPool,
}

impl PostgresStaffRoleStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl StaffRoleStore for PostgresStaffRoleStore {
    async fn list_active_for_user(&self, user_id: Uuid) -> Result<StaffRoleSet, StaffRoleError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT role FROM staff_roles
             WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        let roles = rows.into_iter().filter_map(|(r,)| r.parse().ok());
        Ok(StaffRoleSet::from_iter(roles))
    }

    async fn grant(
        &self,
        user_id: Uuid,
        role: StaffRole,
        granted_by: Option<Uuid>,
        reason: Option<&str>,
    ) -> Result<bool, StaffRoleError> {
        // Partial unique index `staff_roles_active_uq` enforces "one
        // active grant per (user, role)". `ON CONFLICT DO NOTHING`
        // makes the call idempotent; the inserted row count tells us
        // whether a new grant was created.
        let inserted: Option<(Uuid,)> = sqlx::query_as(
            "INSERT INTO staff_roles (id, user_id, role, granted_by_user_id, reason)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (user_id, role) WHERE revoked_at IS NULL DO NOTHING
             RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(role.as_str())
        .bind(granted_by)
        .bind(reason)
        .fetch_optional(&self.pool)
        .await?;

        Ok(inserted.is_some())
    }

    async fn revoke(
        &self,
        user_id: Uuid,
        role: StaffRole,
        revoked_by: Option<Uuid>,
    ) -> Result<bool, StaffRoleError> {
        // UPDATE the single active row (per the partial unique
        // index). RETURNING id lets us tell "we revoked something"
        // apart from "no active grant existed" without a follow-up
        // SELECT.
        let updated: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE staff_roles
             SET revoked_at = NOW(), revoked_by_user_id = $1
             WHERE user_id = $2 AND role = $3 AND revoked_at IS NULL
             RETURNING id",
        )
        .bind(revoked_by)
        .bind(user_id)
        .bind(role.as_str())
        .fetch_optional(&self.pool)
        .await?;

        Ok(updated.is_some())
    }
}

/// Read STARSTATS_BOOTSTRAP_ADMIN_HANDLES, look each handle up, and
/// idempotently grant the `admin` role. Each new grant emits one
/// `audit_log` row with `actor_sub = NULL` and
/// `action = "admin.bootstrap.grant"`.
///
/// Failures for individual handles (handle empty, user not found,
/// audit-log error) are logged and do NOT abort startup -- a typo in
/// the env var shouldn't keep the server from booting. A failed DB
/// call against the staff_roles table itself is bubbled up because
/// it likely means the migration didn't run.
pub async fn bootstrap_admins_from_env<U, S>(
    users: &U,
    store: &S,
    audit: &dyn AuditLog,
    env_var_name: &str,
) -> Result<(), StaffRoleError>
where
    U: UserStore,
    S: StaffRoleStore,
{
    let raw = match std::env::var(env_var_name) {
        Ok(v) => v,
        Err(_) => {
            tracing::debug!(
                env_var = env_var_name,
                "staff_roles bootstrap: env var not set, skipping"
            );
            return Ok(());
        }
    };

    let handles: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if handles.is_empty() {
        tracing::warn!(
            env_var = env_var_name,
            "staff_roles bootstrap: env var set but contains no handles"
        );
        return Ok(());
    }

    for handle in handles {
        match users.find_by_handle(handle).await {
            Ok(Some(user)) => {
                grant_one(store, audit, &user, handle).await?;
            }
            Ok(None) => {
                // User hasn't signed up yet; log so it's visible but
                // don't fail. They can sign up later and the next
                // restart will pick them up.
                tracing::warn!(
                    handle = %handle,
                    "staff_roles bootstrap: handle not found, will retry on next startup"
                );
            }
            Err(e) => {
                tracing::error!(
                    handle = %handle,
                    err = ?e,
                    "staff_roles bootstrap: user lookup failed"
                );
            }
        }
    }

    Ok(())
}

async fn grant_one<S: StaffRoleStore>(
    store: &S,
    audit: &dyn AuditLog,
    user: &User,
    handle: &str,
) -> Result<(), StaffRoleError> {
    let inserted = store
        .grant(
            user.id,
            StaffRole::Admin,
            None,
            Some("bootstrap from STARSTATS_BOOTSTRAP_ADMIN_HANDLES"),
        )
        .await?;

    if inserted {
        tracing::info!(
            handle = %handle,
            user_id = %user.id,
            "staff_roles bootstrap: granted admin"
        );

        // Audit-log the grant. `actor_sub = None` marks it as a system
        // action; the payload carries the handle + user id so the
        // event is self-describing without joining anything.
        // Audit is best-effort (project invariant): a failed append must not
        // abort the bootstrap loop and leave later admins ungranted. The grant
        // itself already succeeded above (M-S1).
        if let Err(e) = audit
            .append(AuditEntry {
                actor_sub: None,
                actor_handle: None,
                action: "admin.bootstrap.grant".to_owned(),
                payload: json!({
                    "user_id": user.id,
                    "handle": handle,
                    "role": StaffRole::Admin.as_str(),
                }),
            })
            .await
        {
            tracing::warn!(
                error = %e,
                handle = %handle,
                user_id = %user.id,
                "staff_roles bootstrap: audit append failed (grant applied)"
            );
        }
    } else {
        tracing::debug!(
            handle = %handle,
            user_id = %user.id,
            "staff_roles bootstrap: already admin, skipping"
        );
    }

    Ok(())
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// In-memory store for tests. Mirrors the `staff_roles` table
    /// closely enough that the auth extractor and bootstrap routine
    /// behave identically against either backend.
    #[derive(Default)]
    pub struct MemoryStaffRoleStore {
        rows: Mutex<Vec<StaffRoleGrant>>,
    }

    impl MemoryStaffRoleStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl StaffRoleStore for MemoryStaffRoleStore {
        async fn list_active_for_user(
            &self,
            user_id: Uuid,
        ) -> Result<StaffRoleSet, StaffRoleError> {
            let rows = self.rows.lock().unwrap();
            let roles = rows.iter().filter(|g| g.user_id == user_id).map(|g| g.role);
            Ok(StaffRoleSet::from_iter(roles))
        }

        async fn grant(
            &self,
            user_id: Uuid,
            role: StaffRole,
            granted_by: Option<Uuid>,
            reason: Option<&str>,
        ) -> Result<bool, StaffRoleError> {
            let mut rows = self.rows.lock().unwrap();
            if rows.iter().any(|g| g.user_id == user_id && g.role == role) {
                return Ok(false);
            }
            rows.push(StaffRoleGrant {
                id: Uuid::new_v4(),
                user_id,
                role,
                granted_at: Utc::now(),
                granted_by_user_id: granted_by,
                reason: reason.map(str::to_owned),
            });
            Ok(true)
        }

        async fn revoke(
            &self,
            user_id: Uuid,
            role: StaffRole,
            _revoked_by: Option<Uuid>,
        ) -> Result<bool, StaffRoleError> {
            // The memory store has no concept of "soft delete"; a
            // revoke is a remove. Tests checking "is the role
            // active after revoke?" still get the right answer.
            let mut rows = self.rows.lock().unwrap();
            let before = rows.len();
            rows.retain(|g| !(g.user_id == user_id && g.role == role));
            Ok(rows.len() < before)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryStaffRoleStore;
    use super::*;
    use crate::audit::{AuditEntry, AuditError, AuditLog};
    use crate::users::test_support::MemoryUserStore;
    use std::sync::Mutex;

    #[derive(Default)]
    struct CapturingAuditLog {
        entries: Mutex<Vec<AuditEntry>>,
    }

    #[async_trait]
    impl AuditLog for CapturingAuditLog {
        async fn append(&self, entry: AuditEntry) -> Result<(), AuditError> {
            self.entries.lock().unwrap().push(entry);
            Ok(())
        }
    }

    #[tokio::test]
    async fn role_set_admin_implies_moderator() {
        let only_admin = StaffRoleSet::from_iter([StaffRole::Admin]);
        assert!(only_admin.has(StaffRole::Admin));
        assert!(only_admin.has(StaffRole::Moderator));

        let only_mod = StaffRoleSet::from_iter([StaffRole::Moderator]);
        assert!(!only_mod.has(StaffRole::Admin));
        assert!(only_mod.has(StaffRole::Moderator));

        let none = StaffRoleSet::new();
        assert!(!none.has(StaffRole::Admin));
        assert!(!none.has(StaffRole::Moderator));
    }

    #[tokio::test]
    async fn role_str_roundtrip() {
        for r in [StaffRole::Moderator, StaffRole::Admin] {
            assert_eq!(StaffRole::from_str(r.as_str()).unwrap(), r);
        }
        assert!(StaffRole::from_str("nonsense").is_err());
    }

    #[tokio::test]
    async fn grant_is_idempotent() {
        let store = MemoryStaffRoleStore::new();
        let user_id = Uuid::new_v4();

        let first = store
            .grant(user_id, StaffRole::Admin, None, None)
            .await
            .unwrap();
        assert!(first, "first grant should report inserted=true");

        let second = store
            .grant(user_id, StaffRole::Admin, None, None)
            .await
            .unwrap();
        assert!(!second, "second grant should report inserted=false");

        let active = store.list_active_for_user(user_id).await.unwrap();
        assert!(active.has(StaffRole::Admin));
    }

    #[tokio::test]
    async fn bootstrap_grants_existing_handle_and_audit_logs() {
        let users = MemoryUserStore::new();
        let alice = users
            .create("alice@example.com", "phc$dummy", "alice")
            .await
            .unwrap();
        let store = MemoryStaffRoleStore::new();
        let audit = CapturingAuditLog::default();

        let env_key = "STARSTATS_TEST_BOOTSTRAP_HANDLES_A";
        // SAFETY: tests use unique env var names to avoid cross-test
        // pollution; the unsafe set/remove pair is the std API.
        unsafe {
            std::env::set_var(env_key, "alice");
        }

        bootstrap_admins_from_env(&users, &store, &audit, env_key)
            .await
            .unwrap();

        let active = store.list_active_for_user(alice.id).await.unwrap();
        assert!(active.has(StaffRole::Admin));

        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "admin.bootstrap.grant");
        assert!(entries[0].actor_sub.is_none());

        unsafe {
            std::env::remove_var(env_key);
        }
    }

    #[tokio::test]
    async fn bootstrap_is_idempotent_across_runs() {
        let users = MemoryUserStore::new();
        users
            .create("bob@example.com", "phc$dummy", "bob")
            .await
            .unwrap();
        let store = MemoryStaffRoleStore::new();
        let audit = CapturingAuditLog::default();

        let env_key = "STARSTATS_TEST_BOOTSTRAP_HANDLES_B";
        unsafe {
            std::env::set_var(env_key, "bob");
        }

        bootstrap_admins_from_env(&users, &store, &audit, env_key)
            .await
            .unwrap();
        bootstrap_admins_from_env(&users, &store, &audit, env_key)
            .await
            .unwrap();

        // One active grant, one audit row -- the second pass is a
        // no-op because the partial unique index already covers it.
        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);

        unsafe {
            std::env::remove_var(env_key);
        }
    }

    #[tokio::test]
    async fn bootstrap_skips_unknown_handle_without_failing() {
        let users = MemoryUserStore::new();
        let store = MemoryStaffRoleStore::new();
        let audit = CapturingAuditLog::default();

        let env_key = "STARSTATS_TEST_BOOTSTRAP_HANDLES_C";
        unsafe {
            std::env::set_var(env_key, "ghost,alice");
        }

        // alice doesn't exist either; the function shouldn't bail.
        bootstrap_admins_from_env(&users, &store, &audit, env_key)
            .await
            .expect("missing handles must not abort startup");

        let entries = audit.entries.lock().unwrap();
        assert!(entries.is_empty());

        unsafe {
            std::env::remove_var(env_key);
        }
    }

    #[tokio::test]
    async fn bootstrap_handles_comma_separated_list() {
        let users = MemoryUserStore::new();
        let alice = users
            .create("alice@example.com", "phc$dummy", "alice")
            .await
            .unwrap();
        let bob = users
            .create("bob@example.com", "phc$dummy", "bob")
            .await
            .unwrap();
        let store = MemoryStaffRoleStore::new();
        let audit = CapturingAuditLog::default();

        let env_key = "STARSTATS_TEST_BOOTSTRAP_HANDLES_D";
        unsafe {
            std::env::set_var(env_key, " alice , bob ");
        }

        bootstrap_admins_from_env(&users, &store, &audit, env_key)
            .await
            .unwrap();

        assert!(store
            .list_active_for_user(alice.id)
            .await
            .unwrap()
            .has(StaffRole::Admin));
        assert!(store
            .list_active_for_user(bob.id)
            .await
            .unwrap()
            .has(StaffRole::Admin));

        unsafe {
            std::env::remove_var(env_key);
        }
    }

    #[tokio::test]
    async fn bootstrap_unset_env_is_no_op() {
        let users = MemoryUserStore::new();
        let store = MemoryStaffRoleStore::new();
        let audit = CapturingAuditLog::default();

        // Use a name that's almost certainly unset.
        bootstrap_admins_from_env(&users, &store, &audit, "STARSTATS_TEST_NEVER_SET_E")
            .await
            .unwrap();

        let entries = audit.entries.lock().unwrap();
        assert!(entries.is_empty());
    }
}
