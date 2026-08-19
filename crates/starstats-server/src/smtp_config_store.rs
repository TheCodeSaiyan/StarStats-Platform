//! DB-backed SMTP configuration with KEK-encrypted password.
//!
//! Singleton row (`smtp_config.id = 1`, enforced by a `CHECK` in
//! migration `0020_smtp_config.sql`). The password is encrypted with
//! [`crate::kek::Kek`] (the same envelope used for TOTP secrets) and
//! stored as a `(ciphertext, nonce)` BYTEA pair. `Kek::encrypt` is
//! AES-256-GCM — see `kek.rs:80`.
//!
//! The store is consumed by:
//!
//!  * boot-time loader (`main.rs`) — checks if `enabled` is true; if
//!    so, builds a `LettreMailer` from the record and wraps it in the
//!    swappable mailer handle.
//!  * admin routes (`smtp_admin_routes.rs`) — `GET` returns the record
//!    minus the password (with a `password_set` bool), `PUT` persists a
//!    new record and triggers a hot reload.
//!
//! The record returned by `get()` carries the *plaintext* password
//! after decryption. Callers must not log it. The `Debug` impl below
//! redacts the password to keep it out of `tracing::error` diagnostic
//! dumps.

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::PgPool;
use std::fmt;
use uuid::Uuid;

use crate::kek::Kek;

/// In-memory representation of the SMTP config row. The `password`
/// field holds the *plaintext* (only ever in memory) — the on-disk
/// shape lives in two BYTEA columns and is never exposed outside this
/// module.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpConfigRecord {
    pub host: String,
    pub port: i32,
    pub username: String,
    /// Plaintext password. `None` means "no auth" — valid for private
    /// relays. When persisting via `put()` the caller passes `None` to
    /// signal "leave the existing encrypted password unchanged" and
    /// `Some(String::new())` to explicitly clear it.
    pub password: Option<String>,
    pub secure: bool,
    pub from_addr: String,
    pub from_name: String,
    pub web_origin: String,
    pub enabled: bool,
}

// Redact the password in any diagnostic dump. The plaintext only ever
// exists in memory for the duration of a request — we still take care
// not to let it leak into `tracing` payloads.
impl fmt::Debug for SmtpConfigRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpConfigRecord")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[redacted]"))
            .field("secure", &self.secure)
            .field("from_addr", &self.from_addr)
            .field("from_name", &self.from_name)
            .field("web_origin", &self.web_origin)
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[async_trait]
pub trait SmtpConfigStore: Send + Sync + 'static {
    /// Load the singleton row and decrypt the password if set.
    /// Returns the seeded default (`enabled = false`, blank fields)
    /// when the row exists but has never been edited — the migration
    /// guarantees the row is always present.
    async fn get(&self, kek: &Kek) -> Result<SmtpConfigRecord>;

    /// Persist `record`, encrypting the password under `kek` before
    /// the write. `updated_by` is the admin user ID; passing `None`
    /// represents a system-driven write (currently unused — every
    /// real caller is admin-driven).
    ///
    /// When `record.password.is_none()` the existing encrypted
    /// password is preserved. Pass `Some(String::new())` to clear
    /// auth entirely.
    async fn put(
        &self,
        record: SmtpConfigRecord,
        kek: &Kek,
        updated_by: Option<Uuid>,
    ) -> Result<()>;
}

pub struct PostgresSmtpConfigStore {
    pool: PgPool,
}

impl PostgresSmtpConfigStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SmtpConfigStore for PostgresSmtpConfigStore {
    async fn get(&self, kek: &Kek) -> Result<SmtpConfigRecord> {
        let row: (
            String,
            i32,
            String,
            Option<Vec<u8>>,
            Option<Vec<u8>>,
            bool,
            String,
            String,
            String,
            bool,
        ) = sqlx::query_as(
            "SELECT host, port, username, password_ciphertext, password_nonce,
                    secure, from_addr, from_name, web_origin, enabled
             FROM smtp_config WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await
        .context("SELECT smtp_config")?;

        let (
            host,
            port,
            username,
            ct_opt,
            nonce_opt,
            secure,
            from_addr,
            from_name,
            web_origin,
            enabled,
        ) = row;

        let password = match (ct_opt, nonce_opt) {
            (Some(ct), Some(nonce)) => {
                let plain = kek.decrypt(&ct, &nonce).context("decrypt smtp password")?;
                Some(String::from_utf8(plain).context("smtp password not utf-8 after decrypt")?)
            }
            (None, None) => None,
            // Guarded by the smtp_config_password_pair CHECK, but
            // surface a clear error if the constraint ever drifts.
            _ => anyhow::bail!("smtp_config password ciphertext/nonce pair is half-NULL"),
        };

        Ok(SmtpConfigRecord {
            host,
            port,
            username,
            password,
            secure,
            from_addr,
            from_name,
            web_origin,
            enabled,
        })
    }

    async fn put(
        &self,
        record: SmtpConfigRecord,
        kek: &Kek,
        updated_by: Option<Uuid>,
    ) -> Result<()> {
        // Three cases for the password columns:
        //   None                  → keep existing
        //   Some("")              → clear (both columns to NULL)
        //   Some(non-empty)       → encrypt + write fresh nonce
        let (ct_param, nonce_param, keep_existing) = match record.password.as_deref() {
            None => (None, None, true),
            Some("") => (None, None, false),
            Some(plain) => {
                let (ct, nonce) = kek
                    .encrypt(plain.as_bytes())
                    .context("encrypt smtp password")?;
                (Some(ct), Some(nonce), false)
            }
        };

        if keep_existing {
            sqlx::query(
                "UPDATE smtp_config
                 SET host = $1, port = $2, username = $3,
                     secure = $4, from_addr = $5, from_name = $6,
                     web_origin = $7, enabled = $8,
                     updated_at = now(), updated_by = $9
                 WHERE id = 1",
            )
            .bind(&record.host)
            .bind(record.port)
            .bind(&record.username)
            .bind(record.secure)
            .bind(&record.from_addr)
            .bind(&record.from_name)
            .bind(&record.web_origin)
            .bind(record.enabled)
            .bind(updated_by)
            .execute(&self.pool)
            .await
            .context("UPDATE smtp_config (keep existing password)")?;
        } else {
            sqlx::query(
                "UPDATE smtp_config
                 SET host = $1, port = $2, username = $3,
                     password_ciphertext = $4, password_nonce = $5,
                     secure = $6, from_addr = $7, from_name = $8,
                     web_origin = $9, enabled = $10,
                     updated_at = now(), updated_by = $11
                 WHERE id = 1",
            )
            .bind(&record.host)
            .bind(record.port)
            .bind(&record.username)
            .bind(ct_param)
            .bind(nonce_param)
            .bind(record.secure)
            .bind(&record.from_addr)
            .bind(&record.from_name)
            .bind(&record.web_origin)
            .bind(record.enabled)
            .bind(updated_by)
            .execute(&self.pool)
            .await
            .context("UPDATE smtp_config (with password)")?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    /// In-memory `SmtpConfigStore` for unit tests. Mirrors the
    /// encrypt-on-write / decrypt-on-read behaviour of the Postgres
    /// store so tests catch crypto-pairing bugs in the same shape as
    /// production would.
    pub struct MemorySmtpConfigStore {
        inner: Mutex<StoredRow>,
    }

    struct StoredRow {
        host: String,
        port: i32,
        username: String,
        password_ct: Option<Vec<u8>>,
        password_nonce: Option<Vec<u8>>,
        secure: bool,
        from_addr: String,
        from_name: String,
        web_origin: String,
        enabled: bool,
    }

    impl Default for MemorySmtpConfigStore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MemorySmtpConfigStore {
        pub fn new() -> Self {
            Self {
                inner: Mutex::new(StoredRow {
                    host: String::new(),
                    port: 587,
                    username: String::new(),
                    password_ct: None,
                    password_nonce: None,
                    secure: true,
                    from_addr: "noreply@starstats.local".to_string(),
                    from_name: "StarStats".to_string(),
                    web_origin: String::new(),
                    enabled: false,
                }),
            }
        }
    }

    #[async_trait]
    impl SmtpConfigStore for MemorySmtpConfigStore {
        async fn get(&self, kek: &Kek) -> Result<SmtpConfigRecord> {
            let row = self.inner.lock().expect("memory smtp store mutex poisoned");
            let password = match (&row.password_ct, &row.password_nonce) {
                (Some(ct), Some(n)) => {
                    let plain = kek
                        .decrypt(ct, n)
                        .context("decrypt smtp password (memory)")?;
                    Some(String::from_utf8(plain).context("smtp password not utf-8")?)
                }
                (None, None) => None,
                _ => anyhow::bail!("memory store password pair is half-NULL"),
            };
            Ok(SmtpConfigRecord {
                host: row.host.clone(),
                port: row.port,
                username: row.username.clone(),
                password,
                secure: row.secure,
                from_addr: row.from_addr.clone(),
                from_name: row.from_name.clone(),
                web_origin: row.web_origin.clone(),
                enabled: row.enabled,
            })
        }

        async fn put(
            &self,
            record: SmtpConfigRecord,
            kek: &Kek,
            _updated_by: Option<Uuid>,
        ) -> Result<()> {
            let mut row = self.inner.lock().expect("memory smtp store mutex poisoned");
            match record.password.as_deref() {
                None => { /* keep existing */ }
                Some("") => {
                    row.password_ct = None;
                    row.password_nonce = None;
                }
                Some(p) => {
                    let (ct, n) = kek.encrypt(p.as_bytes()).context("encrypt (memory)")?;
                    row.password_ct = Some(ct);
                    row.password_nonce = Some(n);
                }
            }
            row.host = record.host;
            row.port = record.port;
            row.username = record.username;
            row.secure = record.secure;
            row.from_addr = record.from_addr;
            row.from_name = record.from_name;
            row.web_origin = record.web_origin;
            row.enabled = record.enabled;
            Ok(())
        }
    }

    fn test_kek() -> Kek {
        // Unique path per test so `load_or_generate` doesn't trip
        // the "wrong-length file" check by re-reading a prior test's
        // key. The file is left behind in temp_dir — fine for tests.
        let path = std::env::temp_dir().join(format!("starstats-test-kek-{}.bin", Uuid::new_v4()));
        // autogen=true: tests want a fresh key minted at the unique path.
        Kek::load_or_generate(&path, true).expect("load kek")
    }

    #[tokio::test]
    async fn round_trips_password_via_kek() {
        let kek = test_kek();
        let store = MemorySmtpConfigStore::new();

        let rec = SmtpConfigRecord {
            host: "smtp.example.com".into(),
            port: 587,
            username: "apikey".into(),
            password: Some("super-secret-pass".into()),
            secure: true,
            from_addr: "noreply@app.example".into(),
            from_name: "StarStats".into(),
            web_origin: "https://app.example".into(),
            enabled: true,
        };

        store.put(rec.clone(), &kek, None).await.expect("put");

        let loaded = store.get(&kek).await.expect("get");
        assert_eq!(loaded.password.as_deref(), Some("super-secret-pass"));
        assert_eq!(loaded.host, "smtp.example.com");
        assert!(loaded.enabled);
    }

    #[tokio::test]
    async fn put_with_none_password_keeps_existing() {
        let kek = test_kek();
        let store = MemorySmtpConfigStore::new();

        let initial = SmtpConfigRecord {
            host: "smtp.one".into(),
            port: 587,
            username: "u".into(),
            password: Some("original".into()),
            secure: true,
            from_addr: "a@b".into(),
            from_name: "S".into(),
            web_origin: "https://x".into(),
            enabled: true,
        };
        store.put(initial, &kek, None).await.expect("seed");

        // Edit other fields, leave password as None (= "don't touch").
        let edit = SmtpConfigRecord {
            host: "smtp.two".into(),
            port: 465,
            username: "u2".into(),
            password: None,
            secure: false,
            from_addr: "a@b".into(),
            from_name: "S".into(),
            web_origin: "https://x".into(),
            enabled: true,
        };
        store.put(edit, &kek, None).await.expect("edit");

        let loaded = store.get(&kek).await.expect("get");
        assert_eq!(loaded.host, "smtp.two");
        assert_eq!(loaded.port, 465);
        assert!(!loaded.secure);
        assert_eq!(
            loaded.password.as_deref(),
            Some("original"),
            "None password on edit must preserve the original"
        );
    }

    #[tokio::test]
    async fn put_with_empty_password_clears_it() {
        let kek = test_kek();
        let store = MemorySmtpConfigStore::new();

        let initial = SmtpConfigRecord {
            host: "smtp.one".into(),
            port: 587,
            username: "u".into(),
            password: Some("original".into()),
            secure: true,
            from_addr: "a@b".into(),
            from_name: "S".into(),
            web_origin: "https://x".into(),
            enabled: true,
        };
        store.put(initial, &kek, None).await.expect("seed");

        let clear = SmtpConfigRecord {
            host: "smtp.one".into(),
            port: 587,
            username: "".into(),
            password: Some(String::new()),
            secure: true,
            from_addr: "a@b".into(),
            from_name: "S".into(),
            web_origin: "https://x".into(),
            enabled: false,
        };
        store.put(clear, &kek, None).await.expect("clear");

        let loaded = store.get(&kek).await.expect("get");
        assert!(
            loaded.password.is_none(),
            "empty-string password must clear to NULL"
        );
        assert!(!loaded.enabled);
    }

    #[test]
    fn debug_redacts_password() {
        let rec = SmtpConfigRecord {
            host: "smtp.example.com".into(),
            port: 587,
            username: "u".into(),
            password: Some("super-secret-pass".into()),
            secure: true,
            from_addr: "a@b".into(),
            from_name: "S".into(),
            web_origin: "https://x".into(),
            enabled: true,
        };
        let s = format!("{rec:?}");
        assert!(
            !s.contains("super-secret-pass"),
            "Debug must redact password, got: {s}"
        );
        assert!(s.contains("[redacted]"), "expected redaction marker");
    }
}
