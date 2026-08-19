//! Desktop client device pairing.
//!
//! The web user clicks "Pair desktop" on the /devices page. The
//! server emits a short pairing code. The desktop client prompts the
//! user for that code and POSTs to `/v1/auth/devices/redeem`. On
//! success, the desktop client receives a long-lived device JWT and a
//! row is committed to the `devices` table so the user can revoke it
//! later from the same /devices page.
//!
//! Pairing codes are 8 chars from a confusion-free alphabet — read
//! aloud, typed by hand, generally exposed to one user once. They
//! are NOT a security boundary on their own: redemption is gated
//! behind possession of the pairing record's user_id (set when the
//! user generated the code while authenticated).

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use sqlx::PgPool;
use uuid::Uuid;

/// Default lifetime for a freshly issued pairing code.
pub const PAIRING_TTL: Duration = Duration::minutes(5);

/// Default device label when the user doesn't supply one.
pub const DEFAULT_LABEL: &str = "Desktop client";

#[derive(Debug, Clone)]
pub struct Pairing {
    pub code: String,
    /// Returned for completeness — the bin target's start handler
    /// only surfaces code/expires_at/label. Tests use it to assert
    /// the row was created under the right user.
    #[allow(dead_code)]
    pub user_id: Uuid,
    pub label: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RedeemedDevice {
    pub device_id: Uuid,
    pub user_id: Uuid,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: Uuid,
    /// Returned for completeness — the list handler scopes by
    /// caller's user_id so it doesn't surface the field, but tests
    /// and any future admin endpoint will want it.
    #[allow(dead_code)]
    pub user_id: Uuid,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub sync_enabled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("pairing code not found")]
    UnknownCode,
    #[error("pairing code already redeemed")]
    AlreadyRedeemed,
    #[error("pairing code expired")]
    Expired,
    #[error("device not found or not owned by caller")]
    DeviceNotFound,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Three-state device row classification returned by
/// [`DeviceStore::device_auth_status`]. Separates "actually revoked"
/// from "row not found" so the auth middleware can emit distinct
/// 401 error messages — without this, a deleted device row (FK
/// cascade from user delete, manual DELETE) is indistinguishable
/// from an explicit revocation in operator logs and client responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAuthStatus {
    Active,
    Revoked,
    Missing,
}

#[async_trait]
pub trait DeviceStore: Send + Sync + 'static {
    /// Persist a pairing code under `user_id`. Returns the code; the
    /// caller already knows the rest of the row from the inputs.
    async fn create_pairing(
        &self,
        user_id: Uuid,
        label: &str,
        ttl: Duration,
    ) -> Result<Pairing, DeviceError>;

    /// Atomically claim a pairing code and create the corresponding
    /// devices row. Idempotency is critical here — a flaky network
    /// retry from the desktop client should NOT mint two devices.
    /// We achieve this by setting `redeemed_at` on the pairing row
    /// only on the first call; subsequent calls observe it set and
    /// return [`DeviceError::AlreadyRedeemed`].
    async fn redeem(&self, code: &str) -> Result<RedeemedDevice, DeviceError>;

    /// Distinguish device-row states for the auth middleware.
    /// Called on every request that arrives bearing a device JWT —
    /// must stay cheap (single indexed lookup).
    ///
    /// The three states are deliberately differentiated:
    /// - [`DeviceAuthStatus::Active`] — row exists, `revoked_at IS NULL`.
    /// - [`DeviceAuthStatus::Revoked`] — row exists, `revoked_at IS NOT NULL`.
    /// - [`DeviceAuthStatus::Missing`] — no row matches `device_id`.
    ///
    /// The previous bool-returning API collapsed Revoked and Missing
    /// into a single "not active" response, which made it impossible
    /// to diagnose a token whose backing row had been deleted (FK
    /// cascade from user delete, manual DELETE, etc.) versus one that
    /// had been explicitly revoked.
    async fn device_auth_status(&self, device_id: Uuid) -> Result<DeviceAuthStatus, DeviceError>;

    /// Active devices owned by `user_id`, newest first.
    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<Device>, DeviceError>;

    /// Mark a device as revoked. Idempotent — revoking an
    /// already-revoked row is a no-op. Returns
    /// [`DeviceError::DeviceNotFound`] if the device doesn't exist
    /// or belongs to a different user.
    async fn revoke(&self, user_id: Uuid, device_id: Uuid) -> Result<(), DeviceError>;

    /// Revoke every active device for `user_id`. Used by the
    /// password-reset flow to force re-pairing after a credential
    /// change. Idempotent: returns `Ok(0)` when there's nothing to
    /// revoke.
    async fn revoke_all_for_user(&self, user_id: Uuid) -> Result<u64, DeviceError>;

    /// Stamp `last_seen_at = NOW()` on the device row. Called from
    /// the auth extractor on every device-authed request — that's
    /// the canonical "tray is alive" signal that drives the web UI's
    /// online/offline badge (`isDeviceOnline()` in
    /// `apps/web/src/app/devices/page.tsx` treats anything within
    /// the last 5 minutes as online).
    ///
    /// Idempotent — an unknown `device_id` silently succeeds with
    /// zero rows touched. Errors here are best-effort at the call
    /// site: a hiccup MUST NOT fail the underlying request.
    async fn touch_last_seen(&self, device_id: Uuid) -> Result<(), DeviceError>;

    /// Check whether a device's sync gate is enabled. Returns
    /// `Ok(Some(true|false))` when the device exists and is owned by
    /// `user_id`; `Ok(None)` when the device doesn't exist or isn't
    /// owned by the caller (used to map to a 404 at the route layer).
    async fn sync_enabled_for(
        &self,
        user_id: Uuid,
        device_id: Uuid,
    ) -> Result<Option<bool>, DeviceError>;

    /// Set a device's sync gate. Returns DeviceNotFound when the
    /// device doesn't exist or isn't owned by `user_id`.
    async fn set_sync_enabled(
        &self,
        user_id: Uuid,
        device_id: Uuid,
        enabled: bool,
    ) -> Result<(), DeviceError>;
}

// -- Code generator ---------------------------------------------------

const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
//                        ^   ^^^   ^^   ^^^   ^^^^^^^^^^^^^^
//                       skip I    L,O   1,0   (no confusables)

/// Generate an 8-char pairing code from the confusion-free alphabet.
pub fn fresh_code() -> String {
    let mut rng = rand::thread_rng();
    (0..8)
        .map(|_| {
            let idx = rng.gen_range(0..ALPHABET.len());
            ALPHABET[idx] as char
        })
        .collect()
}

// -- Postgres impl ----------------------------------------------------

pub struct PostgresDeviceStore {
    pool: PgPool,
}

impl PostgresDeviceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DeviceStore for PostgresDeviceStore {
    async fn create_pairing(
        &self,
        user_id: Uuid,
        label: &str,
        ttl: Duration,
    ) -> Result<Pairing, DeviceError> {
        // Loop on the rare collision (5 chars × 31 alphabet = ~10^11
        // collisions are vanishingly unlikely, but the table is
        // PRIMARY KEY on `code` so we can't ignore them).
        let label = if label.trim().is_empty() {
            DEFAULT_LABEL.to_owned()
        } else {
            label.to_owned()
        };
        let expires_at = Utc::now() + ttl;
        for _ in 0..5 {
            let code = fresh_code();
            let res = sqlx::query(
                r#"
                INSERT INTO device_pairings (code, user_id, label, expires_at)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(&code)
            .bind(user_id)
            .bind(&label)
            .bind(expires_at)
            .execute(&self.pool)
            .await;
            match res {
                Ok(_) => {
                    return Ok(Pairing {
                        code,
                        user_id,
                        label,
                        expires_at,
                    });
                }
                Err(sqlx::Error::Database(db)) if db.is_unique_violation() => continue,
                Err(e) => return Err(DeviceError::Database(e)),
            }
        }
        Err(DeviceError::Database(sqlx::Error::Protocol(
            "exhausted pairing code retries".into(),
        )))
    }

    async fn redeem(&self, code: &str) -> Result<RedeemedDevice, DeviceError> {
        let mut tx = self.pool.begin().await?;

        // Pessimistic lock — two concurrent redeems must serialise.
        let row: Option<(Uuid, String, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
            r#"
            SELECT user_id, label, expires_at, redeemed_at
            FROM device_pairings
            WHERE code = $1
            FOR UPDATE
            "#,
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await?;

        let (user_id, label, expires_at, redeemed_at) = row.ok_or(DeviceError::UnknownCode)?;

        if redeemed_at.is_some() {
            return Err(DeviceError::AlreadyRedeemed);
        }
        if Utc::now() > expires_at {
            return Err(DeviceError::Expired);
        }

        let device_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO devices (id, user_id, label)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(device_id)
        .bind(user_id)
        .bind(&label)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            UPDATE device_pairings
            SET redeemed_at = NOW()
            WHERE code = $1
            "#,
        )
        .bind(code)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(RedeemedDevice {
            device_id,
            user_id,
            label,
        })
    }

    async fn device_auth_status(&self, device_id: Uuid) -> Result<DeviceAuthStatus, DeviceError> {
        let row: Option<(Option<DateTime<Utc>>,)> = sqlx::query_as(
            r#"
            SELECT revoked_at
            FROM devices
            WHERE id = $1
            "#,
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match row {
            None => DeviceAuthStatus::Missing,
            Some((None,)) => DeviceAuthStatus::Active,
            Some((Some(_),)) => DeviceAuthStatus::Revoked,
        })
    }

    async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<Device>, DeviceError> {
        let rows: Vec<(
            Uuid,
            Uuid,
            String,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
            bool,
        )> = sqlx::query_as(
            r#"
                SELECT id, user_id, label, created_at, last_seen_at, sync_enabled
                FROM devices
                WHERE user_id = $1 AND revoked_at IS NULL
                ORDER BY created_at DESC
                "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(id, user_id, label, created_at, last_seen_at, sync_enabled)| Device {
                    id,
                    user_id,
                    label,
                    created_at,
                    last_seen_at,
                    sync_enabled,
                },
            )
            .collect())
    }

    async fn revoke(&self, user_id: Uuid, device_id: Uuid) -> Result<(), DeviceError> {
        let res = sqlx::query(
            r#"
            UPDATE devices
            SET revoked_at = COALESCE(revoked_at, NOW())
            WHERE id = $1 AND user_id = $2
            "#,
        )
        .bind(device_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(DeviceError::DeviceNotFound);
        }
        Ok(())
    }

    async fn revoke_all_for_user(&self, user_id: Uuid) -> Result<u64, DeviceError> {
        let res = sqlx::query(
            r#"
            UPDATE devices
            SET revoked_at = COALESCE(revoked_at, NOW())
            WHERE user_id = $1 AND revoked_at IS NULL
            "#,
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    async fn touch_last_seen(&self, device_id: Uuid) -> Result<(), DeviceError> {
        // No revoked_at guard — touching a revoked row is harmless
        // (the auth extractor short-circuits on revoked tokens BEFORE
        // calling this), and the cheaper PK-only WHERE clause avoids
        // a partial-index lookup on every authed request.
        sqlx::query("UPDATE devices SET last_seen_at = NOW() WHERE id = $1")
            .bind(device_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn sync_enabled_for(
        &self,
        user_id: Uuid,
        device_id: Uuid,
    ) -> Result<Option<bool>, DeviceError> {
        let row: Option<(bool,)> = sqlx::query_as(
            "SELECT sync_enabled FROM devices \
             WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(device_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DeviceError::Database)?;
        Ok(row.map(|(v,)| v))
    }

    async fn set_sync_enabled(
        &self,
        user_id: Uuid,
        device_id: Uuid,
        enabled: bool,
    ) -> Result<(), DeviceError> {
        let result = sqlx::query(
            "UPDATE devices SET sync_enabled = $3 \
             WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(device_id)
        .bind(user_id)
        .bind(enabled)
        .execute(&self.pool)
        .await
        .map_err(DeviceError::Database)?;
        if result.rows_affected() == 0 {
            return Err(DeviceError::DeviceNotFound);
        }
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
    struct Inner {
        pairings: HashMap<String, PairingRow>,
        devices: HashMap<Uuid, DeviceRow>,
    }

    #[derive(Clone)]
    struct PairingRow {
        user_id: Uuid,
        label: String,
        expires_at: DateTime<Utc>,
        redeemed_at: Option<DateTime<Utc>>,
    }

    #[derive(Clone)]
    struct DeviceRow {
        id: Uuid,
        user_id: Uuid,
        label: String,
        created_at: DateTime<Utc>,
        last_seen_at: Option<DateTime<Utc>>,
        revoked_at: Option<DateTime<Utc>>,
        sync_enabled: bool,
    }

    pub struct MemoryDeviceStore {
        inner: Mutex<Inner>,
    }

    impl Default for MemoryDeviceStore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MemoryDeviceStore {
        pub fn new() -> Self {
            Self {
                inner: Mutex::new(Inner::default()),
            }
        }

        /// Test-only: force a pairing into the past so we can assert
        /// the expiry path without sleeping for real.
        pub fn expire_now(&self, code: &str) {
            let mut inner = self.inner.lock().unwrap();
            if let Some(row) = inner.pairings.get_mut(code) {
                row.expires_at = Utc::now() - Duration::seconds(1);
            }
        }

        /// Test-only: insert a paired (non-revoked) device directly,
        /// bypassing the pairing code flow. Returns the new device_id.
        /// `sync_enabled` starts as `false` — call `set_sync_enabled`
        /// afterwards to flip it.
        pub async fn seed_paired_device(
            &self,
            user_id: Uuid,
            label: &str,
        ) -> Result<Uuid, DeviceError> {
            let id = Uuid::new_v4();
            let mut inner = self.inner.lock().unwrap();
            inner.devices.insert(
                id,
                DeviceRow {
                    id,
                    user_id,
                    label: label.to_owned(),
                    created_at: Utc::now(),
                    last_seen_at: None,
                    revoked_at: None,
                    sync_enabled: false,
                },
            );
            Ok(id)
        }
    }

    #[async_trait]
    impl DeviceStore for MemoryDeviceStore {
        async fn create_pairing(
            &self,
            user_id: Uuid,
            label: &str,
            ttl: Duration,
        ) -> Result<Pairing, DeviceError> {
            let mut inner = self.inner.lock().unwrap();
            let label = if label.trim().is_empty() {
                DEFAULT_LABEL.to_owned()
            } else {
                label.to_owned()
            };
            let expires_at = Utc::now() + ttl;

            // Deterministic-ish retry, same shape as the postgres impl.
            for _ in 0..5 {
                let code = fresh_code();
                if let std::collections::hash_map::Entry::Vacant(e) =
                    inner.pairings.entry(code.clone())
                {
                    e.insert(PairingRow {
                        user_id,
                        label: label.clone(),
                        expires_at,
                        redeemed_at: None,
                    });
                    return Ok(Pairing {
                        code,
                        user_id,
                        label,
                        expires_at,
                    });
                }
            }
            Err(DeviceError::Database(sqlx::Error::Protocol(
                "exhausted pairing code retries".into(),
            )))
        }

        async fn redeem(&self, code: &str) -> Result<RedeemedDevice, DeviceError> {
            let mut inner = self.inner.lock().unwrap();
            let row = inner
                .pairings
                .get_mut(code)
                .ok_or(DeviceError::UnknownCode)?;
            if row.redeemed_at.is_some() {
                return Err(DeviceError::AlreadyRedeemed);
            }
            if Utc::now() > row.expires_at {
                return Err(DeviceError::Expired);
            }
            row.redeemed_at = Some(Utc::now());
            let user_id = row.user_id;
            let label = row.label.clone();
            let device_id = Uuid::new_v4();
            inner.devices.insert(
                device_id,
                DeviceRow {
                    id: device_id,
                    user_id,
                    label: label.clone(),
                    created_at: Utc::now(),
                    last_seen_at: None,
                    revoked_at: None,
                    sync_enabled: false,
                },
            );
            Ok(RedeemedDevice {
                device_id,
                user_id,
                label,
            })
        }

        async fn device_auth_status(
            &self,
            device_id: Uuid,
        ) -> Result<DeviceAuthStatus, DeviceError> {
            let inner = self.inner.lock().unwrap();
            Ok(match inner.devices.get(&device_id) {
                None => DeviceAuthStatus::Missing,
                Some(row) if row.revoked_at.is_none() => DeviceAuthStatus::Active,
                Some(_) => DeviceAuthStatus::Revoked,
            })
        }

        async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<Device>, DeviceError> {
            let inner = self.inner.lock().unwrap();
            let mut out: Vec<Device> = inner
                .devices
                .values()
                .filter(|r| r.user_id == user_id && r.revoked_at.is_none())
                .map(|r| Device {
                    id: r.id,
                    user_id: r.user_id,
                    label: r.label.clone(),
                    created_at: r.created_at,
                    last_seen_at: r.last_seen_at,
                    sync_enabled: r.sync_enabled,
                })
                .collect();
            out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(out)
        }

        async fn revoke(&self, user_id: Uuid, device_id: Uuid) -> Result<(), DeviceError> {
            let mut inner = self.inner.lock().unwrap();
            let row = inner
                .devices
                .get_mut(&device_id)
                .filter(|r| r.user_id == user_id);
            let Some(row) = row else {
                return Err(DeviceError::DeviceNotFound);
            };
            if row.revoked_at.is_none() {
                row.revoked_at = Some(Utc::now());
            }
            Ok(())
        }

        async fn revoke_all_for_user(&self, user_id: Uuid) -> Result<u64, DeviceError> {
            let mut inner = self.inner.lock().unwrap();
            let mut count = 0u64;
            for row in inner.devices.values_mut() {
                if row.user_id == user_id && row.revoked_at.is_none() {
                    row.revoked_at = Some(Utc::now());
                    count += 1;
                }
            }
            Ok(count)
        }

        async fn touch_last_seen(&self, device_id: Uuid) -> Result<(), DeviceError> {
            let mut inner = self.inner.lock().unwrap();
            if let Some(row) = inner.devices.get_mut(&device_id) {
                row.last_seen_at = Some(Utc::now());
            }
            // Unknown device_id is a silent no-op — matches the
            // Postgres impl's "WHERE id = $1 (no rows)" behaviour.
            Ok(())
        }

        async fn sync_enabled_for(
            &self,
            user_id: Uuid,
            device_id: Uuid,
        ) -> Result<Option<bool>, DeviceError> {
            let inner = self.inner.lock().unwrap();
            let result = inner
                .devices
                .get(&device_id)
                .filter(|r| r.user_id == user_id && r.revoked_at.is_none())
                .map(|r| r.sync_enabled);
            Ok(result)
        }

        async fn set_sync_enabled(
            &self,
            user_id: Uuid,
            device_id: Uuid,
            enabled: bool,
        ) -> Result<(), DeviceError> {
            let mut inner = self.inner.lock().unwrap();
            let row = inner
                .devices
                .get_mut(&device_id)
                .filter(|r| r.user_id == user_id && r.revoked_at.is_none());
            match row {
                Some(r) => {
                    r.sync_enabled = enabled;
                    Ok(())
                }
                None => Err(DeviceError::DeviceNotFound),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryDeviceStore;
    use super::*;

    #[test]
    fn fresh_code_is_eight_alphanumeric() {
        for _ in 0..50 {
            let c = fresh_code();
            assert_eq!(c.len(), 8);
            assert!(c.chars().all(|ch| ALPHABET.contains(&(ch as u8))));
        }
    }

    #[tokio::test]
    async fn create_then_redeem_round_trips() {
        let store = MemoryDeviceStore::new();
        let user = Uuid::new_v4();
        let pairing = store
            .create_pairing(user, "Daisy's PC", PAIRING_TTL)
            .await
            .unwrap();
        assert_eq!(pairing.user_id, user);
        let redeemed = store.redeem(&pairing.code).await.unwrap();
        assert_eq!(redeemed.user_id, user);
        assert_eq!(redeemed.label, "Daisy's PC");
    }

    #[tokio::test]
    async fn redeem_is_single_use() {
        let store = MemoryDeviceStore::new();
        let user = Uuid::new_v4();
        let pairing = store
            .create_pairing(user, "Daisy's PC", PAIRING_TTL)
            .await
            .unwrap();
        store.redeem(&pairing.code).await.unwrap();
        let err = store.redeem(&pairing.code).await.unwrap_err();
        assert!(matches!(err, DeviceError::AlreadyRedeemed));
    }

    #[tokio::test]
    async fn unknown_code_is_rejected() {
        let store = MemoryDeviceStore::new();
        let err = store.redeem("ZZZZZZZZ").await.unwrap_err();
        assert!(matches!(err, DeviceError::UnknownCode));
    }

    #[tokio::test]
    async fn expired_pairing_cannot_be_redeemed() {
        let store = MemoryDeviceStore::new();
        let user = Uuid::new_v4();
        let pairing = store
            .create_pairing(user, "Daisy's PC", PAIRING_TTL)
            .await
            .unwrap();
        store.expire_now(&pairing.code);
        let err = store.redeem(&pairing.code).await.unwrap_err();
        assert!(matches!(err, DeviceError::Expired));
    }

    #[tokio::test]
    async fn empty_label_falls_back_to_default() {
        let store = MemoryDeviceStore::new();
        let user = Uuid::new_v4();
        let pairing = store
            .create_pairing(user, "   ", PAIRING_TTL)
            .await
            .unwrap();
        assert_eq!(pairing.label, DEFAULT_LABEL);
    }

    #[tokio::test]
    async fn touch_last_seen_populates_then_advances_the_timestamp() {
        // Regression coverage for the "web UI says offline" bug. The
        // column existed and the UI read it, but no code path ever
        // wrote to it — so every device looked offline. The auth
        // extractor now calls touch_last_seen on every device-authed
        // request; this test pins the contract.
        let store = MemoryDeviceStore::new();
        let user = Uuid::new_v4();
        let pairing = store
            .create_pairing(user, "tray", PAIRING_TTL)
            .await
            .unwrap();
        let redeemed = store.redeem(&pairing.code).await.unwrap();
        // Freshly redeemed: last_seen_at starts as None — the device
        // exists but no auth request has hit it yet.
        let devices = store.list_for_user(user).await.unwrap();
        assert_eq!(devices.len(), 1);
        assert!(devices[0].last_seen_at.is_none());

        // After touch: populated with a recent timestamp.
        store.touch_last_seen(redeemed.device_id).await.unwrap();
        let after_first = store
            .list_for_user(user)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .last_seen_at
            .expect("first touch should populate last_seen_at");
        // Sanity check the timestamp is recent (within 5s) — guards
        // against accidentally writing some sentinel time.
        let delta = (Utc::now() - after_first).num_seconds().abs();
        assert!(delta < 5, "last_seen_at should be ~now, was {delta}s off");

        // Subsequent touch advances the timestamp monotonically.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        store.touch_last_seen(redeemed.device_id).await.unwrap();
        let after_second = store
            .list_for_user(user)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .last_seen_at
            .unwrap();
        assert!(
            after_second > after_first,
            "second touch should advance the timestamp"
        );
    }

    #[tokio::test]
    async fn touch_last_seen_unknown_device_is_a_silent_noop() {
        // Mirrors the Postgres impl's "WHERE id = $1 (zero rows)"
        // behaviour. An unknown device_id must not produce an error
        // — the auth extractor relies on this so a stale JWT for a
        // deleted device-row can't fail the request twice (once on
        // the revocation check, once on the touch).
        let store = MemoryDeviceStore::new();
        let result = store.touch_last_seen(Uuid::new_v4()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn device_auth_status_distinguishes_active_revoked_missing() {
        // Regression guard for the auth-middleware diagnosability bug:
        // the old `is_device_active(bool)` API collapsed "row exists
        // but revoked" and "row absent" into a single `false`, so a
        // device JWT whose backing row had been DELETED (FK cascade
        // from user delete, manual DELETE) was indistinguishable in
        // operator logs from one explicitly revoked via the web UI.
        // `device_auth_status` returns a 3-state instead.
        let store = MemoryDeviceStore::new();
        let user = Uuid::new_v4();
        let pairing = store
            .create_pairing(user, "tray", PAIRING_TTL)
            .await
            .unwrap();
        let redeemed = store.redeem(&pairing.code).await.unwrap();

        // Fresh redeem → Active.
        assert_eq!(
            store.device_auth_status(redeemed.device_id).await.unwrap(),
            DeviceAuthStatus::Active,
        );
        // After explicit revoke → Revoked (row still exists).
        store.revoke(user, redeemed.device_id).await.unwrap();
        assert_eq!(
            store.device_auth_status(redeemed.device_id).await.unwrap(),
            DeviceAuthStatus::Revoked,
        );

        // Unknown id → Missing (no row at all). Both surfaces map to
        // a 401 in the middleware, but the error string differs
        // ("device not found" vs "device revoked") so operators can
        // tell which case fired without a DB query.
        let unknown = Uuid::new_v4();
        assert_eq!(
            store.device_auth_status(unknown).await.unwrap(),
            DeviceAuthStatus::Missing,
        );
    }
}
