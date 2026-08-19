//! Public-beta signup gate: the waitlist queue and its admission rule.
//!
//! Admission is "auto-admit while COUNT(admitted) < cap". That is a
//! count-then-insert, which Postgres will happily let two concurrent
//! transactions both win -- `FOR UPDATE` on existing rows does not block a
//! concurrent INSERT, so it cannot serialise an append (the same shape
//! that bit the audit-log hash chain). We take a `pg_advisory_xact_lock`
//! over a fixed key for the whole decision instead. Signups are rare; the
//! lock costs nothing and the alternative is quietly admitting cap+N
//! people, with no error anywhere to say so.
//!
//! For the same reason the cap is read from `waitlist_config` per signup
//! rather than mirrored into an atomic. The ship-matrix media kill-switch
//! uses an `AtomicBool` because the media proxy is a per-request hot path;
//! waitlist signup is not, so the DB read buys simplicity and costs
//! nothing measurable. Copying that pattern here would be cargo-culting.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

/// Fixed key for the admission advisory lock. Arbitrary but stable --
/// every admission path must use this same value or the lock is useless.
/// ("WAITLST\x01" as big-endian bytes.)
const WAITLIST_ADMISSION_LOCK: i64 = 0x5741_4954_4C53_5401;

#[derive(Debug, thiserror::Error)]
pub enum WaitlistError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignupOutcome {
    Admitted { invite_token: String },
    Queued { position: i64 },
    AlreadyAdmitted,
    AlreadyQueued { position: i64 },
}

#[derive(Debug, Clone)]
pub struct WaitlistEntry {
    pub id: Uuid,
    pub email: String,
    pub source: Option<String>,
    pub created_at: DateTime<Utc>,
    pub admitted_at: Option<DateTime<Utc>>,
    /// When the invite was redeemed (an account now exists), or `None`.
    /// Surfaced so the admin console can badge the row and disable its
    /// delete checkbox BEFORE ever calling `delete_batch` — a correct
    /// refusal from that SQL predicate must not read as a dead button.
    /// This field is only a hint: `delete_batch`'s predicate remains the
    /// sole authority, so a row that looks unredeemed here can still come
    /// back in `blocked` if it was redeemed a moment later.
    pub invite_consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct AdmittedInvite {
    pub email: String,
    pub invite_token: String,
}

/// Result of a delete batch. Two lists rather than a count so the console
/// can say WHICH rows were refused — "deleted 3 of 4" leaves an admin
/// guessing which one survived and why.
#[derive(Debug, Clone, Default)]
pub struct DeleteOutcome {
    /// Rows actually removed.
    pub deleted: Vec<Uuid>,
    /// Rows refused because the invite was already redeemed. Ids that
    /// never existed appear in NEITHER list.
    pub blocked: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueStatus {
    Queued,
    Admitted,
}

#[async_trait]
pub trait WaitlistStore: Send + Sync + 'static {
    async fn signup(
        &self,
        email: &str,
        source: Option<&str>,
    ) -> Result<SignupOutcome, WaitlistError>;
    async fn list(
        &self,
        status: QueueStatus,
        limit: i64,
    ) -> Result<Vec<WaitlistEntry>, WaitlistError>;
    async fn admit_batch(&self, ids: &[Uuid]) -> Result<Vec<AdmittedInvite>, WaitlistError>;
    /// Re-send targets for already-admitted rows: their email + the
    /// EXISTING invite token, read without mutation. This is the resend
    /// path — an admitted row whose invite email failed to send (an SMTP
    /// outage during auto-admit, say) can be re-mailed the same live
    /// link. Read-only by construction, so unlike a re-admit it cannot
    /// invalidate a link already in an inbox. Rows that are not admitted,
    /// or somehow carry no token, are skipped.
    async fn resend_batch(&self, ids: &[Uuid]) -> Result<Vec<AdmittedInvite>, WaitlistError>;
    /// Permanently delete signups. Rows whose invite was already redeemed
    /// (`invite_consumed_at IS NOT NULL`) are REFUSED, not deleted: that
    /// invite produced a real account, and erasing the row would destroy
    /// the only record the person was ever invited. Unknown ids are a
    /// no-op. Deleting frees the UNIQUE email for re-use, and frees a cap
    /// slot when the row was admitted.
    async fn delete_batch(&self, ids: &[Uuid]) -> Result<DeleteOutcome, WaitlistError>;
    /// Consume an invite. `Some(id)` on success, `None` if unknown or
    /// already consumed. Idempotent-safe: a second call returns `None`.
    async fn redeem_invite(&self, token: &str) -> Result<Option<Uuid>, WaitlistError>;
    /// Un-consume an invite. Called when signup fails AFTER redeeming --
    /// picking a taken handle must not eject someone from the beta.
    async fn release_invite(&self, token: &str) -> Result<(), WaitlistError>;
    async fn cap(&self) -> Result<i64, WaitlistError>;
    async fn gate_enabled(&self) -> Result<bool, WaitlistError>;
    async fn set_config(
        &self,
        cap: i64,
        gate_enabled: bool,
        updated_by: Option<Uuid>,
    ) -> Result<(), WaitlistError>;
}

/// 64 hex chars from 32 random bytes -- same shape as the magic-link and
/// verification tokens.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn normalise_email(email: &str) -> String {
    email.trim().to_lowercase()
}

pub struct PostgresWaitlistStore {
    pool: PgPool,
}

impl PostgresWaitlistStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WaitlistStore for PostgresWaitlistStore {
    async fn signup(
        &self,
        email: &str,
        source: Option<&str>,
    ) -> Result<SignupOutcome, WaitlistError> {
        let email = normalise_email(email);
        let mut tx = self.pool.begin().await?;

        // Serialise the whole count-then-decide. See module docs: without
        // this, two concurrent signups both read cap-1 and both admit.
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(WAITLIST_ADMISSION_LOCK)
            .execute(&mut *tx)
            .await?;

        let existing: Option<(Uuid, Option<DateTime<Utc>>)> =
            sqlx::query_as("SELECT id, admitted_at FROM waitlist_signups WHERE email = $1")
                .bind(&email)
                .fetch_optional(&mut *tx)
                .await?;

        if let Some((id, admitted_at)) = existing {
            let outcome = if admitted_at.is_some() {
                SignupOutcome::AlreadyAdmitted
            } else {
                SignupOutcome::AlreadyQueued {
                    position: position_of(&mut tx, id).await?,
                }
            };
            tx.commit().await?;
            return Ok(outcome);
        }

        let (cap,): (i32,) = sqlx::query_as("SELECT cap FROM waitlist_config WHERE id = 1")
            .fetch_one(&mut *tx)
            .await?;
        let (admitted,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM waitlist_signups WHERE admitted_at IS NOT NULL")
                .fetch_one(&mut *tx)
                .await?;

        let outcome = if admitted < i64::from(cap) {
            let token = generate_token();
            sqlx::query(
                "INSERT INTO waitlist_signups (email, source, admitted_at, invite_token) \
                 VALUES ($1, $2, NOW(), $3)",
            )
            .bind(&email)
            .bind(source)
            .bind(&token)
            .execute(&mut *tx)
            .await?;
            SignupOutcome::Admitted {
                invite_token: token,
            }
        } else {
            let (id,): (Uuid,) = sqlx::query_as(
                "INSERT INTO waitlist_signups (email, source) VALUES ($1, $2) RETURNING id",
            )
            .bind(&email)
            .bind(source)
            .fetch_one(&mut *tx)
            .await?;
            SignupOutcome::Queued {
                position: position_of(&mut tx, id).await?,
            }
        };

        tx.commit().await?;
        Ok(outcome)
    }

    async fn list(
        &self,
        status: QueueStatus,
        limit: i64,
    ) -> Result<Vec<WaitlistEntry>, WaitlistError> {
        let sql = match status {
            QueueStatus::Queued => {
                "SELECT id, email, source, created_at, admitted_at, invite_consumed_at \
                 FROM waitlist_signups \
                 WHERE admitted_at IS NULL ORDER BY created_at ASC LIMIT $1"
            }
            QueueStatus::Admitted => {
                "SELECT id, email, source, created_at, admitted_at, invite_consumed_at \
                 FROM waitlist_signups \
                 WHERE admitted_at IS NOT NULL ORDER BY admitted_at DESC LIMIT $1"
            }
        };
        let rows: Vec<(
            Uuid,
            String,
            Option<String>,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(sql)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(id, email, source, created_at, admitted_at, invite_consumed_at)| WaitlistEntry {
                    id,
                    email,
                    source,
                    created_at,
                    admitted_at,
                    invite_consumed_at,
                },
            )
            .collect())
    }

    async fn admit_batch(&self, ids: &[Uuid]) -> Result<Vec<AdmittedInvite>, WaitlistError> {
        let mut out = Vec::new();
        for id in ids {
            let token = generate_token();
            // `AND admitted_at IS NULL` makes this idempotent: an admin
            // double-click must not re-mint an invite over a live one and
            // silently invalidate the link already in someone's inbox.
            let row: Option<(String,)> = sqlx::query_as(
                "UPDATE waitlist_signups SET admitted_at = NOW(), invite_token = $2 \
                 WHERE id = $1 AND admitted_at IS NULL RETURNING email",
            )
            .bind(id)
            .bind(&token)
            .fetch_optional(&self.pool)
            .await?;
            if let Some((email,)) = row {
                out.push(AdmittedInvite {
                    email,
                    invite_token: token,
                });
            }
        }
        Ok(out)
    }

    async fn resend_batch(&self, ids: &[Uuid]) -> Result<Vec<AdmittedInvite>, WaitlistError> {
        // Read-only: no UPDATE, so the live token is untouched and any
        // link already in an inbox stays valid. `= ANY($1)` over the id
        // array keeps it a single round trip.
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT email, invite_token FROM waitlist_signups \
             WHERE id = ANY($1) AND admitted_at IS NOT NULL AND invite_token IS NOT NULL",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(email, invite_token)| AdmittedInvite {
                email,
                invite_token,
            })
            .collect())
    }

    async fn delete_batch(&self, ids: &[Uuid]) -> Result<DeleteOutcome, WaitlistError> {
        // The guard is the DELETE's own predicate, NOT a read-then-delete
        // pre-check: a pre-check could pass and have the invite redeemed
        // before the DELETE landed, deleting a row that became protected
        // in between. As a predicate that race cannot exist.
        //
        // The UNION branch reads the pre-statement snapshot, but the two
        // predicates are disjoint (`IS NULL` vs `IS NOT NULL`) so no row
        // can appear in both halves.
        let rows: Vec<(Uuid, bool)> = sqlx::query_as(
            "WITH del AS ( \
                 DELETE FROM waitlist_signups \
                  WHERE id = ANY($1) AND invite_consumed_at IS NULL \
                 RETURNING id \
             ) \
             SELECT id, TRUE AS deleted FROM del \
             UNION ALL \
             SELECT id, FALSE FROM waitlist_signups \
              WHERE id = ANY($1) AND invite_consumed_at IS NOT NULL",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;

        let mut out = DeleteOutcome::default();
        for (id, deleted) in rows {
            if deleted {
                out.deleted.push(id);
            } else {
                out.blocked.push(id);
            }
        }
        Ok(out)
    }

    async fn redeem_invite(&self, token: &str) -> Result<Option<Uuid>, WaitlistError> {
        // UPDATE ... RETURNING marks consumed and reads in one round trip,
        // closing the lookup-then-write race. Same shape as magic_link.rs.
        let row: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE waitlist_signups SET invite_consumed_at = NOW() \
             WHERE invite_token = $1 AND invite_consumed_at IS NULL AND admitted_at IS NOT NULL \
             RETURNING id",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    async fn release_invite(&self, token: &str) -> Result<(), WaitlistError> {
        sqlx::query(
            "UPDATE waitlist_signups SET invite_consumed_at = NULL WHERE invite_token = $1",
        )
        .bind(token)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn cap(&self) -> Result<i64, WaitlistError> {
        let (cap,): (i32,) = sqlx::query_as("SELECT cap FROM waitlist_config WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(i64::from(cap))
    }

    async fn gate_enabled(&self) -> Result<bool, WaitlistError> {
        let (on,): (bool,) =
            sqlx::query_as("SELECT gate_enabled FROM waitlist_config WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        Ok(on)
    }

    async fn set_config(
        &self,
        cap: i64,
        gate_enabled: bool,
        updated_by: Option<Uuid>,
    ) -> Result<(), WaitlistError> {
        sqlx::query(
            "UPDATE waitlist_config SET cap = $1, gate_enabled = $2, updated_at = NOW(), \
             updated_by = $3 WHERE id = 1",
        )
        .bind(i32::try_from(cap).unwrap_or(i32::MAX))
        .bind(gate_enabled)
        .bind(updated_by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// 1-based position among the still-queued, oldest first.
async fn position_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<i64, WaitlistError> {
    let (pos,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) + 1 FROM waitlist_signups \
         WHERE admitted_at IS NULL \
           AND created_at < (SELECT created_at FROM waitlist_signups WHERE id = $1)",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(pos)
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct Row {
        id: Uuid,
        admitted: bool,
        token: Option<String>,
        consumed: bool,
        seq: u64,
    }

    /// In-memory `WaitlistStore` for handler and store tests. Mirrors the
    /// Postgres semantics that matter: lowercase emails, cap-gated
    /// admission, 1-based queue positions, single-use invites.
    pub struct MemoryWaitlistStore {
        rows: Mutex<HashMap<String, Row>>,
        cap: Mutex<i64>,
        gate: Mutex<bool>,
        seq: Mutex<u64>,
    }

    impl MemoryWaitlistStore {
        /// Gate ON with `cap` admissions available. Defaults ON here
        /// (unlike the migration's FALSE) because every test reaching for
        /// this constructor is testing gated behaviour.
        pub fn with_cap(cap: i64) -> Self {
            Self {
                rows: Mutex::new(HashMap::new()),
                cap: Mutex::new(cap),
                gate: Mutex::new(true),
                seq: Mutex::new(0),
            }
        }

        /// Gate OFF — signup open to anyone, invites not required.
        ///
        /// This is production's default (migration 0050 ships
        /// `gate_enabled = FALSE`), so it is what unrelated auth tests
        /// should use: they assert today's behaviour, which the gate must
        /// not disturb until an admin turns it on.
        pub fn open() -> Self {
            Self {
                rows: Mutex::new(HashMap::new()),
                cap: Mutex::new(0),
                gate: Mutex::new(false),
                seq: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl WaitlistStore for MemoryWaitlistStore {
        async fn signup(
            &self,
            email: &str,
            _source: Option<&str>,
        ) -> Result<SignupOutcome, WaitlistError> {
            let email = normalise_email(email);
            let mut rows = self.rows.lock().unwrap();
            if let Some(r) = rows.get(&email) {
                if r.admitted {
                    return Ok(SignupOutcome::AlreadyAdmitted);
                }
                let seq = r.seq;
                let pos = rows.values().filter(|o| !o.admitted && o.seq < seq).count() as i64 + 1;
                return Ok(SignupOutcome::AlreadyQueued { position: pos });
            }
            let admitted_count = rows.values().filter(|r| r.admitted).count() as i64;
            let this_seq = {
                let mut seq = self.seq.lock().unwrap();
                *seq += 1;
                *seq
            };
            let cap = *self.cap.lock().unwrap();
            if admitted_count < cap {
                let token = generate_token();
                rows.insert(
                    email,
                    Row {
                        id: Uuid::new_v4(),
                        admitted: true,
                        token: Some(token.clone()),
                        consumed: false,
                        seq: this_seq,
                    },
                );
                Ok(SignupOutcome::Admitted {
                    invite_token: token,
                })
            } else {
                rows.insert(
                    email,
                    Row {
                        id: Uuid::new_v4(),
                        admitted: false,
                        token: None,
                        consumed: false,
                        seq: this_seq,
                    },
                );
                let pos = rows.values().filter(|r| !r.admitted).count() as i64;
                Ok(SignupOutcome::Queued { position: pos })
            }
        }

        async fn list(
            &self,
            status: QueueStatus,
            _limit: i64,
        ) -> Result<Vec<WaitlistEntry>, WaitlistError> {
            let rows = self.rows.lock().unwrap();
            let want_admitted = matches!(status, QueueStatus::Admitted);
            Ok(rows
                .iter()
                .filter(|(_, r)| r.admitted == want_admitted)
                .map(|(email, r)| WaitlistEntry {
                    id: r.id,
                    email: email.clone(),
                    source: None,
                    created_at: Utc::now(),
                    admitted_at: r.admitted.then(Utc::now),
                    invite_consumed_at: r.consumed.then(Utc::now),
                })
                .collect())
        }

        async fn admit_batch(&self, ids: &[Uuid]) -> Result<Vec<AdmittedInvite>, WaitlistError> {
            let mut rows = self.rows.lock().unwrap();
            let mut out = Vec::new();
            for (email, r) in rows.iter_mut() {
                if ids.contains(&r.id) && !r.admitted {
                    let token = generate_token();
                    r.admitted = true;
                    r.token = Some(token.clone());
                    out.push(AdmittedInvite {
                        email: email.clone(),
                        invite_token: token,
                    });
                }
            }
            Ok(out)
        }

        async fn resend_batch(&self, ids: &[Uuid]) -> Result<Vec<AdmittedInvite>, WaitlistError> {
            let rows = self.rows.lock().unwrap();
            let mut out = Vec::new();
            for (email, r) in rows.iter() {
                if ids.contains(&r.id) && r.admitted {
                    if let Some(token) = &r.token {
                        out.push(AdmittedInvite {
                            email: email.clone(),
                            invite_token: token.clone(),
                        });
                    }
                }
            }
            Ok(out)
        }

        async fn delete_batch(&self, ids: &[Uuid]) -> Result<DeleteOutcome, WaitlistError> {
            let mut rows = self.rows.lock().unwrap();
            let mut out = DeleteOutcome::default();
            let mut remove: Vec<String> = Vec::new();
            for (email, r) in rows.iter() {
                if !ids.contains(&r.id) {
                    continue;
                }
                if r.consumed {
                    out.blocked.push(r.id);
                } else {
                    out.deleted.push(r.id);
                    remove.push(email.clone());
                }
            }
            for email in remove {
                rows.remove(&email);
            }
            Ok(out)
        }

        async fn redeem_invite(&self, token: &str) -> Result<Option<Uuid>, WaitlistError> {
            let mut rows = self.rows.lock().unwrap();
            for r in rows.values_mut() {
                if r.token.as_deref() == Some(token) && !r.consumed && r.admitted {
                    r.consumed = true;
                    return Ok(Some(r.id));
                }
            }
            Ok(None)
        }

        async fn release_invite(&self, token: &str) -> Result<(), WaitlistError> {
            let mut rows = self.rows.lock().unwrap();
            for r in rows.values_mut() {
                if r.token.as_deref() == Some(token) {
                    r.consumed = false;
                }
            }
            Ok(())
        }

        async fn cap(&self) -> Result<i64, WaitlistError> {
            Ok(*self.cap.lock().unwrap())
        }

        async fn gate_enabled(&self) -> Result<bool, WaitlistError> {
            Ok(*self.gate.lock().unwrap())
        }

        async fn set_config(
            &self,
            cap: i64,
            gate_enabled: bool,
            _updated_by: Option<Uuid>,
        ) -> Result<(), WaitlistError> {
            *self.cap.lock().unwrap() = cap;
            *self.gate.lock().unwrap() = gate_enabled;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryWaitlistStore;
    use super::*;

    // -- Test 1: the cap actually caps -------------------------------

    #[tokio::test]
    async fn admits_under_cap_and_queues_at_cap() {
        let store = MemoryWaitlistStore::with_cap(2);
        assert!(matches!(
            store.signup("a@example.com", None).await.unwrap(),
            SignupOutcome::Admitted { .. }
        ));
        assert!(matches!(
            store.signup("b@example.com", None).await.unwrap(),
            SignupOutcome::Admitted { .. }
        ));
        // Cap reached -- the third asks in and waits.
        assert!(matches!(
            store.signup("c@example.com", None).await.unwrap(),
            SignupOutcome::Queued { position: 1 }
        ));
    }

    // -- Test 2: resubmitting is not a failure -----------------------

    #[tokio::test]
    async fn resubmitting_returns_existing_position_not_an_error() {
        let store = MemoryWaitlistStore::with_cap(0);
        let first = store.signup("a@example.com", None).await.unwrap();
        let second = store.signup("a@example.com", None).await.unwrap();
        assert!(matches!(first, SignupOutcome::Queued { position: 1 }));
        // Someone who forgot they signed up must not see a failure.
        assert!(matches!(
            second,
            SignupOutcome::AlreadyQueued { position: 1 }
        ));
    }

    #[tokio::test]
    async fn resubmitting_after_admission_reports_already_admitted() {
        let store = MemoryWaitlistStore::with_cap(1);
        let _ = store.signup("a@example.com", None).await.unwrap();
        assert!(matches!(
            store.signup("a@example.com", None).await.unwrap(),
            SignupOutcome::AlreadyAdmitted
        ));
    }

    // -- Test 3: email case ------------------------------------------

    #[tokio::test]
    async fn email_is_normalised_to_lowercase() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("Alice@Example.COM", None).await.unwrap();
        let out = store.signup("alice@example.com", None).await.unwrap();
        assert!(matches!(out, SignupOutcome::AlreadyQueued { .. }));
    }

    #[tokio::test]
    async fn surrounding_whitespace_is_trimmed() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("  a@example.com  ", None).await.unwrap();
        assert!(matches!(
            store.signup("a@example.com", None).await.unwrap(),
            SignupOutcome::AlreadyQueued { .. }
        ));
    }

    // -- Test 4: invites are single-use ------------------------------

    #[tokio::test]
    async fn invite_redeems_once_then_never_again() {
        let store = MemoryWaitlistStore::with_cap(1);
        let SignupOutcome::Admitted { invite_token } =
            store.signup("a@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        assert!(store.redeem_invite(&invite_token).await.unwrap().is_some());
        assert!(store.redeem_invite(&invite_token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unknown_invite_is_none_not_an_error() {
        let store = MemoryWaitlistStore::with_cap(1);
        assert!(store.redeem_invite("nope").await.unwrap().is_none());
    }

    // -- Test 5: release puts a burnt invite back --------------------

    #[tokio::test]
    async fn released_invite_can_be_redeemed_again() {
        let store = MemoryWaitlistStore::with_cap(1);
        let SignupOutcome::Admitted { invite_token } =
            store.signup("a@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        assert!(store.redeem_invite(&invite_token).await.unwrap().is_some());
        // Signup failed downstream (taken handle) -- give the invite back.
        store.release_invite(&invite_token).await.unwrap();
        assert!(store.redeem_invite(&invite_token).await.unwrap().is_some());
    }

    // -- Test 6: queue positions are stable and 1-based --------------

    #[tokio::test]
    async fn queue_positions_are_one_based_and_ordered() {
        let store = MemoryWaitlistStore::with_cap(0);
        assert!(matches!(
            store.signup("a@example.com", None).await.unwrap(),
            SignupOutcome::Queued { position: 1 }
        ));
        assert!(matches!(
            store.signup("b@example.com", None).await.unwrap(),
            SignupOutcome::Queued { position: 2 }
        ));
        // First in line stays first when they check again.
        assert!(matches!(
            store.signup("a@example.com", None).await.unwrap(),
            SignupOutcome::AlreadyQueued { position: 1 }
        ));
    }

    // -- Test 7: config round-trips ----------------------------------

    #[tokio::test]
    async fn set_config_changes_cap_and_gate() {
        let store = MemoryWaitlistStore::with_cap(0);
        assert!(store.gate_enabled().await.unwrap());
        store.set_config(10, false, None).await.unwrap();
        assert_eq!(store.cap().await.unwrap(), 10);
        assert!(!store.gate_enabled().await.unwrap());
        // Raising the cap lets the next signup straight in.
        assert!(matches!(
            store.signup("a@example.com", None).await.unwrap(),
            SignupOutcome::Admitted { .. }
        ));
    }

    // -- Test 8: tokens are not guessable-looking --------------------

    #[tokio::test]
    async fn tokens_are_unique_and_64_hex_chars() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // -- Test 9: the queue read splits by status ---------------------

    #[tokio::test]
    async fn list_separates_queued_from_admitted() {
        let store = MemoryWaitlistStore::with_cap(1);
        store.signup("admitted@example.com", None).await.unwrap();
        store.signup("waiting@example.com", None).await.unwrap();

        let queued = store.list(QueueStatus::Queued, 100).await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].email, "waiting@example.com");
        assert!(queued[0].admitted_at.is_none());

        let admitted = store.list(QueueStatus::Admitted, 100).await.unwrap();
        assert_eq!(admitted.len(), 1);
        assert_eq!(admitted[0].email, "admitted@example.com");
        assert!(admitted[0].admitted_at.is_some());
    }

    // -- Test 9b: list surfaces invite_consumed_at, only for redeemed rows

    #[tokio::test]
    async fn list_reports_invite_consumed_at_for_redeemed_rows_only() {
        let store = MemoryWaitlistStore::with_cap(2);
        let SignupOutcome::Admitted { invite_token } =
            store.signup("redeemed@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        store.signup("live@example.com", None).await.unwrap();
        store.redeem_invite(&invite_token).await.unwrap();

        let rows = store.list(QueueStatus::Admitted, 10).await.unwrap();
        let redeemed = rows
            .iter()
            .find(|r| r.email == "redeemed@example.com")
            .unwrap();
        let live = rows.iter().find(|r| r.email == "live@example.com").unwrap();
        // The console needs this to tell "an account exists, do not offer
        // delete" from "still just an invite" — a badge on the wrong row
        // in either direction is worse than no badge at all.
        assert!(redeemed.invite_consumed_at.is_some());
        assert!(live.invite_consumed_at.is_none());
    }

    // -- Test 10: batch admission ------------------------------------

    #[tokio::test]
    async fn admit_batch_admits_queued_rows_and_returns_their_invites() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("a@example.com", None).await.unwrap();
        store.signup("b@example.com", None).await.unwrap();

        let queued = store.list(QueueStatus::Queued, 100).await.unwrap();
        let ids: Vec<Uuid> = queued.iter().map(|e| e.id).collect();

        let invites = store.admit_batch(&ids).await.unwrap();
        assert_eq!(invites.len(), 2);
        // Every admitted row must come back with a usable invite, or the
        // person is "admitted" with no way in — a silent dead end.
        for inv in &invites {
            assert_eq!(inv.invite_token.len(), 64);
            assert!(store
                .redeem_invite(&inv.invite_token)
                .await
                .unwrap()
                .is_some());
        }
        assert!(store
            .list(QueueStatus::Queued, 100)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn admit_batch_is_idempotent_for_already_admitted_rows() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("a@example.com", None).await.unwrap();
        let ids: Vec<Uuid> = store
            .list(QueueStatus::Queued, 100)
            .await
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();

        assert_eq!(store.admit_batch(&ids).await.unwrap().len(), 1);
        // A double-click must not re-mint over a live invite and silently
        // invalidate the link already sitting in someone's inbox.
        assert_eq!(store.admit_batch(&ids).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn admit_batch_ignores_unknown_ids() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("a@example.com", None).await.unwrap();
        let invites = store.admit_batch(&[Uuid::new_v4()]).await.unwrap();
        assert!(invites.is_empty());
    }

    // -- Test 11: resend re-sends the SAME live invite ----------------

    #[tokio::test]
    async fn resend_batch_returns_the_existing_token_without_reminting() {
        let store = MemoryWaitlistStore::with_cap(1);
        let SignupOutcome::Admitted { invite_token } =
            store.signup("a@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        let id = store.list(QueueStatus::Admitted, 100).await.unwrap()[0].id;

        let resent = store.resend_batch(&[id]).await.unwrap();
        assert_eq!(resent.len(), 1);
        assert_eq!(resent[0].email, "a@example.com");
        // The whole point: the link already in someone's inbox must stay
        // valid, so resend hands back the SAME token, never a fresh one.
        assert_eq!(resent[0].invite_token, invite_token);
        // And the token is still redeemable afterwards — resend neither
        // consumed nor rotated it.
        assert!(store.redeem_invite(&invite_token).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn resend_batch_skips_queued_rows() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("waiting@example.com", None).await.unwrap();
        let id = store.list(QueueStatus::Queued, 100).await.unwrap()[0].id;
        // A queued row has no invite to resend — handing one out would be
        // minting a link for someone who was never admitted.
        assert!(store.resend_batch(&[id]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn resend_batch_ignores_unknown_ids() {
        let store = MemoryWaitlistStore::with_cap(1);
        store.signup("a@example.com", None).await.unwrap();
        assert!(store
            .resend_batch(&[Uuid::new_v4()])
            .await
            .unwrap()
            .is_empty());
    }

    // -- Test 12: delete_batch permanently removes signups -----------

    #[tokio::test]
    async fn delete_batch_removes_a_queued_row_and_frees_its_email() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("Junk@Example.com", None).await.unwrap();
        let queued = store.list(QueueStatus::Queued, 10).await.unwrap();
        let id = queued[0].id;

        let out = store.delete_batch(&[id]).await.unwrap();
        assert_eq!(out.deleted, vec![id]);
        assert!(out.blocked.is_empty());
        assert!(store
            .list(QueueStatus::Queued, 10)
            .await
            .unwrap()
            .is_empty());

        // The point of the feature: the UNIQUE email is free again.
        store.signup("junk@example.com", None).await.unwrap();
        assert_eq!(store.list(QueueStatus::Queued, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_batch_removes_an_admitted_but_unredeemed_row() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("a@example.com", None).await.unwrap();
        let id = store.list(QueueStatus::Queued, 10).await.unwrap()[0].id;
        store.admit_batch(&[id]).await.unwrap();

        let out = store.delete_batch(&[id]).await.unwrap();
        assert_eq!(out.deleted, vec![id]);
        assert!(store
            .list(QueueStatus::Admitted, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn delete_batch_refuses_a_redeemed_row() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("real@example.com", None).await.unwrap();
        let id = store.list(QueueStatus::Queued, 10).await.unwrap()[0].id;
        let invites = store.admit_batch(&[id]).await.unwrap();
        store.redeem_invite(&invites[0].invite_token).await.unwrap();

        let out = store.delete_batch(&[id]).await.unwrap();
        assert!(out.deleted.is_empty(), "a redeemed invite must survive");
        assert_eq!(out.blocked, vec![id]);
        assert_eq!(
            store.list(QueueStatus::Admitted, 10).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn delete_batch_deletes_the_deletable_and_reports_the_rest() {
        let store = MemoryWaitlistStore::with_cap(0);
        store.signup("keep@example.com", None).await.unwrap();
        store.signup("drop@example.com", None).await.unwrap();
        let rows = store.list(QueueStatus::Queued, 10).await.unwrap();
        let keep = rows
            .iter()
            .find(|r| r.email == "keep@example.com")
            .unwrap()
            .id;
        let drop = rows
            .iter()
            .find(|r| r.email == "drop@example.com")
            .unwrap()
            .id;
        let invites = store.admit_batch(&[keep]).await.unwrap();
        store.redeem_invite(&invites[0].invite_token).await.unwrap();

        let out = store.delete_batch(&[keep, drop]).await.unwrap();
        assert_eq!(out.deleted, vec![drop]);
        assert_eq!(out.blocked, vec![keep]);
    }

    #[tokio::test]
    async fn delete_batch_ignores_unknown_ids() {
        let store = MemoryWaitlistStore::with_cap(0);
        let out = store.delete_batch(&[Uuid::new_v4()]).await.unwrap();
        assert!(out.deleted.is_empty());
        assert!(
            out.blocked.is_empty(),
            "unknown is neither deleted nor blocked"
        );
    }

    // -- Test 13: delete_batch against a REAL Postgres (env-gated) ---
    //
    // Every `delete_batch` test above runs against `MemoryWaitlistStore`,
    // so the actual SQL -- the `WITH del AS (DELETE ...) ... UNION ALL
    // ...` predicate split, the real `UNIQUE(email)` constraint, and the
    // `invite_consumed_at` guard -- has never executed against a real
    // database. This test is that missing exercise. Runs ONLY when
    // `STARSTATS_TEST_DATABASE_URL` points at a real Postgres; offline
    // `cargo test` skips it (early return). Run with `--test-threads=1`:
    // this test owns the whole `waitlist_signups` table for its
    // duration and truncates it on entry, so a concurrent test would
    // both lose its rows and have its own rows swept away.
    #[tokio::test]
    async fn delete_batch_against_real_postgres_matches_the_seven_claims() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping waitlist delete_batch PG test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        // `waitlist_config` has a singleton seed row (id = 1) from the
        // migration -- truncate only the signups table, never config.
        sqlx::query("TRUNCATE waitlist_signups")
            .execute(&pool)
            .await
            .expect("truncate waitlist_signups");

        async fn id_for_email(pool: &PgPool, email: &str) -> Uuid {
            sqlx::query_scalar("SELECT id FROM waitlist_signups WHERE email = $1")
                .bind(email)
                .fetch_one(pool)
                .await
                .unwrap_or_else(|e| panic!("no row for {email}: {e}"))
        }

        async fn row_exists(pool: &PgPool, id: Uuid) -> bool {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM waitlist_signups WHERE id = $1")
                    .bind(id)
                    .fetch_one(pool)
                    .await
                    .expect("count by id");
            count > 0
        }

        let store = PostgresWaitlistStore::new(pool.clone());
        // Cap 0: every signup queues. Rows are moved to admitted (and
        // redeemed) explicitly via admit_batch/redeem_invite below, so
        // each claim controls its own row's status precisely.
        store
            .set_config(0, true, None)
            .await
            .expect("set cap 0 for the whole test");

        // -- Claim 1: a queued (never-admitted) row is deleted -----------
        store
            .signup("claim1-queued@example.com", None)
            .await
            .expect("claim1 signup");
        let id1 = id_for_email(&pool, "claim1-queued@example.com").await;

        let out = store
            .delete_batch(&[id1])
            .await
            .expect("claim1 delete_batch");
        assert_eq!(out.deleted, vec![id1], "queued row is deleted");
        assert!(out.blocked.is_empty());
        assert!(
            !row_exists(&pool, id1).await,
            "queued row gone from the table"
        );

        // -- Claim 2: an admitted-but-unredeemed row is deleted ----------
        store
            .signup("claim2-admitted@example.com", None)
            .await
            .expect("claim2 signup");
        let id2 = id_for_email(&pool, "claim2-admitted@example.com").await;
        let invites = store.admit_batch(&[id2]).await.expect("claim2 admit_batch");
        assert_eq!(invites.len(), 1, "claim2 row admitted");

        let out = store
            .delete_batch(&[id2])
            .await
            .expect("claim2 delete_batch");
        assert_eq!(
            out.deleted,
            vec![id2],
            "admitted-but-unredeemed row is deleted"
        );
        assert!(out.blocked.is_empty());
        assert!(
            !row_exists(&pool, id2).await,
            "admitted-but-unredeemed row gone from the table"
        );

        // -- Claim 3: a redeemed row is REFUSED and survives --------------
        store
            .signup("claim3-redeemed@example.com", None)
            .await
            .expect("claim3 signup");
        let id3 = id_for_email(&pool, "claim3-redeemed@example.com").await;
        let invites = store.admit_batch(&[id3]).await.expect("claim3 admit_batch");
        let redeemed = store
            .redeem_invite(&invites[0].invite_token)
            .await
            .expect("claim3 redeem_invite")
            .expect("claim3 redeem succeeds");
        assert_eq!(redeemed, id3);

        let out = store
            .delete_batch(&[id3])
            .await
            .expect("claim3 delete_batch");
        assert!(
            out.deleted.is_empty(),
            "a redeemed invite must survive delete_batch"
        );
        assert_eq!(out.blocked, vec![id3]);
        // Assert survival by RE-QUERYING the table, not just by reading
        // the outcome struct -- the struct could be right for the wrong
        // reason if the DELETE itself silently no-op'd.
        assert!(
            row_exists(&pool, id3).await,
            "redeemed row must still exist in the table after a refused delete"
        );
        let consumed_at: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT invite_consumed_at FROM waitlist_signups WHERE id = $1")
                .bind(id3)
                .fetch_one(&pool)
                .await
                .expect("read invite_consumed_at");
        assert!(
            consumed_at.is_some(),
            "surviving row is still marked redeemed"
        );

        // -- Claim 4 (+ 7): a mixed batch classifies each row correctly,
        // and the UNION ALL branches cannot double-report an id. `id3`
        // (still redeemed, from claim 3) supplies the blocked half; a
        // fresh queued row supplies the deletable half; a never-inserted
        // id supplies the unknown half (also covers claim 5 inline).
        store
            .signup("claim4-deletable@example.com", None)
            .await
            .expect("claim4 signup");
        let deletable_id = id_for_email(&pool, "claim4-deletable@example.com").await;
        let unknown_id = Uuid::new_v4();

        let out = store
            .delete_batch(&[deletable_id, id3, unknown_id])
            .await
            .expect("claim4 delete_batch");
        assert_eq!(
            out.deleted,
            vec![deletable_id],
            "only the deletable row is reported deleted"
        );
        assert_eq!(
            out.blocked,
            vec![id3],
            "only the redeemed row is reported blocked"
        );
        assert!(
            !out.deleted.contains(&unknown_id) && !out.blocked.contains(&unknown_id),
            "the unknown id appears in neither list"
        );

        let deleted_set: std::collections::HashSet<_> = out.deleted.iter().collect();
        let blocked_set: std::collections::HashSet<_> = out.blocked.iter().collect();
        assert!(
            deleted_set.is_disjoint(&blocked_set),
            "no id may appear in both deleted and blocked (UNION ALL double-report)"
        );

        assert!(
            !row_exists(&pool, deletable_id).await,
            "deletable row removed"
        );
        assert!(row_exists(&pool, id3).await, "blocked row still present");

        // -- Claim 5 (standalone): an unknown-only batch is a clean no-op.
        let out = store
            .delete_batch(&[Uuid::new_v4()])
            .await
            .expect("claim5 delete_batch");
        assert!(out.deleted.is_empty());
        assert!(
            out.blocked.is_empty(),
            "an unknown id alone is neither deleted nor blocked, and not an error"
        );

        // -- Claim 6: deleting frees the UNIQUE email for re-signup -------
        // Without the delete, this second signup would hit the real
        // UNIQUE(email) constraint and fail -- that constraint is never
        // exercised by MemoryWaitlistStore, which has no such index.
        let reuse_email = "claim6-reuse@example.com";
        store
            .signup(reuse_email, None)
            .await
            .expect("claim6 first signup");
        let id6 = id_for_email(&pool, reuse_email).await;
        let out = store
            .delete_batch(&[id6])
            .await
            .expect("claim6 delete_batch");
        assert_eq!(out.deleted, vec![id6]);
        assert!(!row_exists(&pool, id6).await);

        let second = store
            .signup(reuse_email, None)
            .await
            .expect("second signup with the freed email must succeed, not hit UNIQUE");
        assert_eq!(
            second,
            SignupOutcome::Queued { position: 1 },
            "freed email re-queues cleanly (no leftover queued rows survive at this point)"
        );

        sqlx::query("TRUNCATE waitlist_signups")
            .execute(&pool)
            .await
            .ok();
    }
}
