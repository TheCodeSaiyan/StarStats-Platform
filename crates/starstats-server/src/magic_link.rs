//! Magic-link sign-in tokens.
//!
//! Single-table primitive: random tokens that, while unconsumed and
//! unexpired, can be redeemed for a session JWT for the user they
//! were issued to. The trait fronts the operations so handlers can
//! be tested without Postgres.
//!
//! Lifetime: 15 minutes from issuance, single-use. The redeemer
//! marks `consumed_at` atomically with the lookup so a click + a
//! refresh-of-the-confirmation-page can't both turn into sessions.
//!
//! There is no rate-limit *here* — the per-IP `tower_governor` layer
//! that `magic_link_routes::routes()` puts on its own router covers
//! magic-start abuse. It has to be built there: a layer applied in
//! `auth_routes::routes()` covers only that router's routes, and this
//! one is merged in as a sibling, so the shared `/v1/auth/` prefix
//! confers nothing. The "always return 200" anti-enumeration policy
//! lives in the route handler, not the store.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

/// Default lifetime of a freshly-issued magic-link token. Long
/// enough that an email round-trip + the user noticing the message
/// fits comfortably; short enough that a forgotten phone with the
/// inbox open doesn't leave a usable token lying around forever.
pub const MAGIC_LINK_TTL: chrono::Duration = chrono::Duration::minutes(15);

#[derive(Debug, thiserror::Error)]
pub enum MagicLinkError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// What `redeem` returns — a successfully-consumed token. The
/// caller mints a session JWT for `user_id`.
#[derive(Debug, Clone)]
pub struct RedeemedMagicLink {
    pub user_id: Uuid,
}

#[async_trait]
pub trait MagicLinkStore: Send + Sync + 'static {
    /// Issue a token for `user_id` valid for `MAGIC_LINK_TTL`.
    /// Returns the token string the caller emails to the user.
    async fn issue(&self, user_id: Uuid) -> Result<String, MagicLinkError>;

    /// Atomically consume a token: mark it used and return the
    /// owning user. Returns `None` for unknown, expired, or already
    /// consumed tokens — the handler collapses all three into
    /// "invalid_or_expired" so a probe can't tell which.
    async fn redeem(&self, token: &str) -> Result<Option<RedeemedMagicLink>, MagicLinkError>;
}

/// 32 random bytes -> 64-char hex token. Same shape as the
/// password-reset and email-verification tokens for predictability;
/// the partial PK on `magic_link_tokens.token` makes lookups O(1).
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 of the token, hex-encoded — what actually goes in the
/// table. The mailed token is the only usable copy; the row is a
/// fingerprint, so reading the table (a backup, a dump, a stray log)
/// no longer hands over a live login.
///
/// No salt and no KDF on purpose: the input is already 256 bits of
/// CSPRNG output, so there is nothing to brute-force and nothing a
/// rainbow table can precompute. `token` stays the primary key —
/// a fingerprint is exactly as unique as what it fingerprints.
///
/// Deploy note: links mailed before this shipped stop redeeming,
/// because their rows hold a raw token that no longer matches any
/// fingerprint. The blast radius is one 15-minute TTL, and the
/// remedy is to request another link.
fn fingerprint(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub struct PostgresMagicLinkStore {
    pool: PgPool,
}

impl PostgresMagicLinkStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MagicLinkStore for PostgresMagicLinkStore {
    async fn issue(&self, user_id: Uuid) -> Result<String, MagicLinkError> {
        let token = generate_token();
        let expires_at: DateTime<Utc> = Utc::now() + MAGIC_LINK_TTL;
        sqlx::query(
            r#"
            INSERT INTO magic_link_tokens (token, user_id, expires_at)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(fingerprint(&token))
        .bind(user_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    async fn redeem(&self, token: &str) -> Result<Option<RedeemedMagicLink>, MagicLinkError> {
        // The UPDATE-with-RETURNING pattern atomically marks the
        // token consumed and returns the row in a single round trip,
        // closing the race window between lookup and write.
        let row: Option<(Uuid,)> = sqlx::query_as(
            r#"
            UPDATE magic_link_tokens
               SET consumed_at = NOW()
             WHERE token = $1
               AND consumed_at IS NULL
               AND expires_at > NOW()
             RETURNING user_id
            "#,
        )
        .bind(fingerprint(token))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(user_id,)| RedeemedMagicLink { user_id }))
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryMagicLinkStore {
        // (token -> (user_id, expires_at, consumed))
        pub rows: Mutex<HashMap<String, (Uuid, DateTime<Utc>, bool)>>,
    }

    impl MemoryMagicLinkStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl MagicLinkStore for MemoryMagicLinkStore {
        async fn issue(&self, user_id: Uuid) -> Result<String, MagicLinkError> {
            let token = generate_token();
            let expires_at = Utc::now() + MAGIC_LINK_TTL;
            self.rows
                .lock()
                .unwrap()
                .insert(fingerprint(&token), (user_id, expires_at, false));
            Ok(token)
        }

        async fn redeem(&self, token: &str) -> Result<Option<RedeemedMagicLink>, MagicLinkError> {
            let mut rows = self.rows.lock().unwrap();
            let entry = rows.get_mut(&fingerprint(token));
            match entry {
                Some((user_id, expires_at, consumed)) if !*consumed && *expires_at > Utc::now() => {
                    *consumed = true;
                    Ok(Some(RedeemedMagicLink { user_id: *user_id }))
                }
                _ => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryMagicLinkStore;
    use super::*;

    /// The row is the credential: anything that can read the table —
    /// a backup, a dump, a stray log — is a 15-minute login for every
    /// unconsumed link. Store a fingerprint and keep the only usable
    /// copy in the mail.
    #[tokio::test]
    async fn the_stored_row_is_not_the_mailed_token() {
        let store = MemoryMagicLinkStore::new();
        let user_id = Uuid::new_v4();
        let mailed = store.issue(user_id).await.expect("issue");

        assert!(
            !store.rows.lock().unwrap().contains_key(&mailed),
            "the mailed token is persisted verbatim — reading the table is a login"
        );
        assert!(
            store.redeem(&mailed).await.expect("redeem").is_some(),
            "the mailed token must still redeem"
        );
    }

    #[tokio::test]
    async fn issue_then_redeem_returns_user() {
        let store = MemoryMagicLinkStore::new();
        let uid = Uuid::new_v4();
        let token = store.issue(uid).await.unwrap();
        let r = store.redeem(&token).await.unwrap().unwrap();
        assert_eq!(r.user_id, uid);
    }

    #[tokio::test]
    async fn second_redeem_returns_none() {
        let store = MemoryMagicLinkStore::new();
        let uid = Uuid::new_v4();
        let token = store.issue(uid).await.unwrap();
        let _ = store.redeem(&token).await.unwrap();
        assert!(store.redeem(&token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn redeem_unknown_returns_none() {
        let store = MemoryMagicLinkStore::new();
        assert!(store.redeem("nope").await.unwrap().is_none());
    }
}
