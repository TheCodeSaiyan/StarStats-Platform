//! Recovery codes for TOTP fallback.
//!
//! Generated as a batch of 10 when a user enables TOTP (or
//! regenerates on demand). Each code is one-shot — verify-and-burn —
//! and stored as an Argon2id hash, the same one-way scheme as
//! passwords. We don't need to display the codes again after
//! issuance, only verify them, so encryption (which would need a
//! reversible key) is the wrong tool.
//!
//! Code format: `XXXX-XXXX-XXXX-XXXX` (16 hex digits with hyphens).
//! 64 bits of entropy is comfortably more than enough — the bound
//! is "user can read and type this" not "brute force is hard,"
//! since the per-IP rate limit on `/v1/auth/*` keeps online attacks
//! to a few attempts per minute regardless of code length.

use crate::users::{hash_password, verify_password};
use async_trait::async_trait;
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

const NUM_CODES: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum RecoveryCodeError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("hash error: {0}")]
    Hash(String),
}

#[async_trait]
pub trait RecoveryCodeStore: Send + Sync + 'static {
    /// Replace any existing codes for `user_id` with a fresh batch.
    /// Returns the plaintext codes — these are the only time the
    /// caller (and the user) ever see them, so the handler must
    /// surface them in the response and emphasise "save these now."
    async fn regenerate_for_user(&self, user_id: Uuid) -> Result<Vec<String>, RecoveryCodeError>;

    /// Verify a candidate code. On match, marks the row used and
    /// returns `true`. Returns `false` for unknown / already-used
    /// codes — collapsing the two so a probe can't tell which.
    async fn redeem(&self, user_id: Uuid, candidate: &str) -> Result<bool, RecoveryCodeError>;

    /// Hard-delete every code for `user_id`. Used by `disable_totp`
    /// and as a defensive cleanup when a user deletes their account
    /// (the FK cascade also handles that, but explicit beats
    /// incidental).
    async fn clear_for_user(&self, user_id: Uuid) -> Result<(), RecoveryCodeError>;
}

/// Generate a single `XXXX-XXXX-XXXX-XXXX` code. Public so the
/// memory test impl + the Postgres impl can share it.
pub fn generate_code() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    let hex = hex::encode(bytes).to_uppercase();
    // hex is 16 chars; insert hyphens at 4-char boundaries.
    format!(
        "{}-{}-{}-{}",
        &hex[0..4],
        &hex[4..8],
        &hex[8..12],
        &hex[12..16]
    )
}

/// Normalise a candidate code for comparison: strip whitespace and
/// uppercase. Lets the user paste with surrounding spaces or in any
/// case without us having to anticipate every variant.
fn normalise(candidate: &str) -> String {
    candidate
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

pub struct PostgresRecoveryCodeStore {
    pool: PgPool,
}

impl PostgresRecoveryCodeStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RecoveryCodeStore for PostgresRecoveryCodeStore {
    async fn regenerate_for_user(&self, user_id: Uuid) -> Result<Vec<String>, RecoveryCodeError> {
        let mut codes = Vec::with_capacity(NUM_CODES);
        let mut hashes = Vec::with_capacity(NUM_CODES);
        for _ in 0..NUM_CODES {
            let code = generate_code();
            let h = hash_password(&code).map_err(|e| RecoveryCodeError::Hash(e.to_string()))?;
            codes.push(code);
            hashes.push(h);
        }

        let mut tx = self.pool.begin().await?;
        // Idempotent regenerate: drop the old set, then insert
        // fresh. The window between delete and insert is inside
        // the transaction so a concurrent verifier sees either
        // the old set or the new one — never an empty middle.
        sqlx::query("DELETE FROM recovery_codes WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        for h in &hashes {
            sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES ($1, $2)")
                .bind(user_id)
                .bind(h)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;

        Ok(codes)
    }

    async fn redeem(&self, user_id: Uuid, candidate: &str) -> Result<bool, RecoveryCodeError> {
        let candidate = normalise(candidate);
        // Pull every unused row for this user; we have to brute-force
        // compare because Argon2 is per-row salted (we can't index by
        // hash). 10 rows max per user, so this is fine.
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, code_hash FROM recovery_codes \
             WHERE user_id = $1 AND used_at IS NULL",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        for (id, hash) in rows {
            if verify_password(&candidate, &hash) {
                // Mark this specific row used. Defensive guard on
                // `used_at IS NULL` so a concurrent redeem of the
                // same code (would never happen — same user double-
                // submitting) loses cleanly.
                let result = sqlx::query(
                    "UPDATE recovery_codes SET used_at = NOW() \
                     WHERE id = $1 AND used_at IS NULL",
                )
                .bind(id)
                .execute(&self.pool)
                .await?;
                return Ok(result.rows_affected() == 1);
            }
        }
        Ok(false)
    }

    async fn clear_for_user(&self, user_id: Uuid) -> Result<(), RecoveryCodeError> {
        sqlx::query("DELETE FROM recovery_codes WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryRecoveryCodeStore {
        // user_id -> Vec<(hash, used)>
        rows: Mutex<HashMap<Uuid, Vec<(String, bool)>>>,
    }

    impl MemoryRecoveryCodeStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl RecoveryCodeStore for MemoryRecoveryCodeStore {
        async fn regenerate_for_user(
            &self,
            user_id: Uuid,
        ) -> Result<Vec<String>, RecoveryCodeError> {
            let mut codes = Vec::with_capacity(NUM_CODES);
            let mut hashes = Vec::with_capacity(NUM_CODES);
            for _ in 0..NUM_CODES {
                let code = generate_code();
                let h = hash_password(&code).map_err(|e| RecoveryCodeError::Hash(e.to_string()))?;
                codes.push(code);
                hashes.push((h, false));
            }
            self.rows.lock().unwrap().insert(user_id, hashes);
            Ok(codes)
        }

        async fn redeem(&self, user_id: Uuid, candidate: &str) -> Result<bool, RecoveryCodeError> {
            let candidate = normalise(candidate);
            let mut rows = self.rows.lock().unwrap();
            let Some(entries) = rows.get_mut(&user_id) else {
                return Ok(false);
            };
            for (hash, used) in entries.iter_mut() {
                if !*used && verify_password(&candidate, hash) {
                    *used = true;
                    return Ok(true);
                }
            }
            Ok(false)
        }

        async fn clear_for_user(&self, user_id: Uuid) -> Result<(), RecoveryCodeError> {
            self.rows.lock().unwrap().remove(&user_id);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryRecoveryCodeStore;
    use super::*;

    #[test]
    fn generate_code_has_expected_shape() {
        let code = generate_code();
        // 4 quartets of 4 hex chars + 3 hyphens = 19 chars
        assert_eq!(code.len(), 19);
        let parts: Vec<&str> = code.split('-').collect();
        assert_eq!(parts.len(), 4);
        for p in parts {
            assert_eq!(p.len(), 4);
            assert!(p.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn normalise_strips_whitespace_and_upper_cases() {
        assert_eq!(normalise(" abcd-1234 "), "ABCD-1234");
        assert_eq!(normalise("ABCD\t-\n1234"), "ABCD-1234");
    }

    #[tokio::test]
    async fn regenerate_returns_ten_unique_codes() {
        let store = MemoryRecoveryCodeStore::new();
        let codes = store.regenerate_for_user(Uuid::new_v4()).await.unwrap();
        assert_eq!(codes.len(), 10);
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), 10);
    }

    #[tokio::test]
    async fn first_redeem_succeeds_second_fails() {
        let store = MemoryRecoveryCodeStore::new();
        let user_id = Uuid::new_v4();
        let codes = store.regenerate_for_user(user_id).await.unwrap();
        let one = codes[0].clone();

        assert!(store.redeem(user_id, &one).await.unwrap());
        assert!(!store.redeem(user_id, &one).await.unwrap());
    }

    #[tokio::test]
    async fn redeem_accepts_lowercase_input() {
        let store = MemoryRecoveryCodeStore::new();
        let user_id = Uuid::new_v4();
        let codes = store.regenerate_for_user(user_id).await.unwrap();
        let lower = codes[0].to_lowercase();
        assert!(store.redeem(user_id, &lower).await.unwrap());
    }

    #[tokio::test]
    async fn regenerate_invalidates_old_set() {
        let store = MemoryRecoveryCodeStore::new();
        let user_id = Uuid::new_v4();
        let first = store.regenerate_for_user(user_id).await.unwrap();
        let _ = store.regenerate_for_user(user_id).await.unwrap();
        // Old code from the first batch should no longer match.
        assert!(!store.redeem(user_id, &first[0]).await.unwrap());
    }

    #[tokio::test]
    async fn clear_for_user_removes_codes() {
        let store = MemoryRecoveryCodeStore::new();
        let user_id = Uuid::new_v4();
        let codes = store.regenerate_for_user(user_id).await.unwrap();
        store.clear_for_user(user_id).await.unwrap();
        assert!(!store.redeem(user_id, &codes[0]).await.unwrap());
    }
}
