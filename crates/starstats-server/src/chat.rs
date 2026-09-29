//! Chat access (social phase 6): who may sign in to the Matrix homeserver,
//! and the short-lived login token that lets them.
//!
//! Chat runs on Synapse (`infra/synapse`), which trusts a JWT signed with a
//! key kept for this alone. The StarStats API mints that token only for a
//! signed-in player with a verified RSI handle, an age declaration against
//! [`CHAT_MIN_AGE`], and no chat restriction; Synapse cannot know any of
//! those, so the gate is here.
//!
//! The token lives [`LOGIN_TOKEN_TTL_SECS`]. Synapse adds a hard-coded 120 s
//! of clock-skew leeway and does not remember used tokens, so a token can
//! sign in for its life plus two minutes; keeping the life short is the
//! only bound on that.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use sqlx::PgPool;

/// The minimum age for chat: the publisher's, since Star Citizen has no
/// formal age rating yet (social design §4.7). One value, so a future
/// rating is one edit, and raising it asks everyone to declare again.
pub const CHAT_MIN_AGE: i16 = 18;

/// How long a Matrix login token is valid, before Synapse's leeway.
pub const LOGIN_TOKEN_TTL_SECS: i64 = 60;

/// The audience Synapse is configured to accept (`jwt_config.audiences`).
pub const LOGIN_TOKEN_AUDIENCE: &str = "matrix";

#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("signing error: {0}")]
    Signing(String),
}

#[async_trait]
pub trait ChatAccessStore: Send + Sync + 'static {
    /// When the player declared, and against which minimum. `None` until
    /// they do.
    async fn age_declaration(
        &self,
        handle: &str,
    ) -> Result<Option<(DateTime<Utc>, i16)>, ChatError>;
    /// Record a declaration against `minimum`. Repeating it keeps the
    /// first date unless the minimum changed.
    async fn declare_age(&self, handle: &str, minimum: i16) -> Result<(), ChatError>;
}

pub struct PostgresChatAccessStore {
    pool: PgPool,
}

impl PostgresChatAccessStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChatAccessStore for PostgresChatAccessStore {
    async fn age_declaration(
        &self,
        handle: &str,
    ) -> Result<Option<(DateTime<Utc>, i16)>, ChatError> {
        let row: Option<(Option<DateTime<Utc>>, Option<i16>)> = sqlx::query_as(
            "SELECT age_declared_at, age_declared_minimum FROM users
             WHERE lower(claimed_handle) = lower($1)",
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match row {
            Some((Some(at), Some(min))) => Some((at, min)),
            _ => None,
        })
    }

    async fn declare_age(&self, handle: &str, minimum: i16) -> Result<(), ChatError> {
        sqlx::query(
            "UPDATE users
             SET age_declared_at = CASE
                     WHEN age_declared_minimum IS NOT DISTINCT FROM $2 AND age_declared_at IS NOT NULL
                     THEN age_declared_at ELSE now() END,
                 age_declared_minimum = $2
             WHERE lower(claimed_handle) = lower($1)",
        )
        .bind(handle)
        .bind(minimum)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Whether a declaration covers the current minimum.
pub fn declared_for_chat(declaration: Option<(DateTime<Utc>, i16)>) -> bool {
    declaration.is_some_and(|(_, min)| min >= CHAT_MIN_AGE)
}

/// Chat's launch switch, `STARSTATS_CHAT_ENABLED`: the same variable, and
/// the same values, the web reads (`apps/web/src/lib/chat/flag.ts`). It
/// decides whether clients OFFER chat, reported as `ChatStatus::offered`
/// so the tray can follow the web without a switch of its own. It is not
/// an access check; the gates in `ChatStatus` are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatLaunch {
    /// Nobody is offered chat. Unset or unknown values land here.
    #[default]
    Off,
    /// Staff only, for testing in production before launch.
    Staff,
    /// Everyone.
    On,
}

impl ChatLaunch {
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("on") => Self::On,
            Some("staff") => Self::Staff,
            _ => Self::Off,
        }
    }

    pub fn from_env() -> Self {
        Self::parse(std::env::var("STARSTATS_CHAT_ENABLED").ok().as_deref())
    }

    /// Whether a caller is offered chat. `is_staff` is any staff grant,
    /// as on the web.
    pub fn offers(self, is_staff: bool) -> bool {
        match self {
            Self::Off => false,
            Self::Staff => is_staff,
            Self::On => true,
        }
    }
}

/// The Matrix localpart for a handle. Synapse accepts `a-z 0-9 _ - . / = +`
/// and does not lowercase, and handles are `[A-Za-z0-9_-]`, so lowercasing
/// is enough. The handle as written becomes the display name.
pub fn localpart(handle: &str) -> String {
    handle.to_ascii_lowercase()
}

#[derive(Debug, Serialize)]
struct LoginClaims<'a> {
    sub: String,
    name: &'a str,
    iss: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

/// Mints Matrix login tokens. Holds the private half of the dedicated
/// matrix login key; Synapse holds the public half.
pub struct MatrixLoginSigner {
    key: EncodingKey,
    issuer: String,
    server_name: String,
    /// Where clients reach the homeserver (its client API base URL).
    pub public_url: String,
}

impl MatrixLoginSigner {
    pub fn new(
        private_key_pem: &str,
        issuer: String,
        server_name: String,
        public_url: String,
    ) -> Result<Self, ChatError> {
        let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .map_err(|e| ChatError::Signing(e.to_string()))?;
        Ok(Self {
            key,
            issuer,
            server_name,
            public_url,
        })
    }

    /// The player's Matrix user ID.
    pub fn user_id(&self, handle: &str) -> String {
        format!("@{}:{}", localpart(handle), self.server_name)
    }

    /// A login token for `handle`, and when it expires.
    pub fn mint(&self, handle: &str, now: DateTime<Utc>) -> Result<(String, i64), ChatError> {
        let iat = now.timestamp();
        let exp = iat + LOGIN_TOKEN_TTL_SECS;
        let claims = LoginClaims {
            sub: localpart(handle),
            name: handle,
            iss: &self.issuer,
            aud: LOGIN_TOKEN_AUDIENCE,
            iat,
            exp,
        };
        let token = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &self.key)
            .map_err(|e| ChatError::Signing(e.to_string()))?;
        Ok((token, exp))
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryChatAccessStore {
        rows: Mutex<HashMap<String, (DateTime<Utc>, i16)>>,
    }

    impl MemoryChatAccessStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl ChatAccessStore for MemoryChatAccessStore {
        async fn age_declaration(
            &self,
            handle: &str,
        ) -> Result<Option<(DateTime<Utc>, i16)>, ChatError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(&handle.to_lowercase())
                .copied())
        }

        async fn declare_age(&self, handle: &str, minimum: i16) -> Result<(), ChatError> {
            let mut rows = self.rows.lock().unwrap();
            let key = handle.to_lowercase();
            let at = match rows.get(&key) {
                Some((at, min)) if *min == minimum => *at,
                _ => Utc::now(),
            };
            rows.insert(key, (at, minimum));
            Ok(())
        }
    }

    /// A signer over a fresh PKCS#8 key (the format `openssl genpkey`
    /// writes, which is what the real key in 1Password is), with the
    /// public half for verifying.
    pub fn signer() -> (MatrixLoginSigner, jsonwebtoken::DecodingKey) {
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        let mut rng = rand::thread_rng();
        let key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
        let private_pem = key.to_pkcs8_pem(LineEnding::LF).expect("pkcs8 pem");
        let public_pem = key
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .expect("public pem");
        let signer = MatrixLoginSigner::new(
            &private_pem,
            "starstats-test".into(),
            "starstats.app".into(),
            "https://api.example/".into(),
        )
        .expect("signer");
        let decoding = jsonwebtoken::DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap();
        (signer, decoding)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use jsonwebtoken::{decode, Validation};

    #[test]
    fn the_launch_switch_reads_like_the_web() {
        assert_eq!(ChatLaunch::parse(None), ChatLaunch::Off);
        assert_eq!(ChatLaunch::parse(Some("off")), ChatLaunch::Off);
        assert_eq!(ChatLaunch::parse(Some("yes")), ChatLaunch::Off);
        assert_eq!(ChatLaunch::parse(Some("staff")), ChatLaunch::Staff);
        assert_eq!(ChatLaunch::parse(Some("on")), ChatLaunch::On);
        assert!(!ChatLaunch::Off.offers(true));
        assert!(ChatLaunch::Staff.offers(true));
        assert!(!ChatLaunch::Staff.offers(false));
        assert!(ChatLaunch::On.offers(false));
    }

    #[test]
    fn a_login_token_carries_what_synapse_checks() {
        let (signer, decoding) = signer();
        let now = Utc::now();
        let (token, exp) = signer.mint("Wing_Man", now).unwrap();
        assert_eq!(exp - now.timestamp(), LOGIN_TOKEN_TTL_SECS);
        let mut v = Validation::new(Algorithm::RS256);
        v.set_audience(&["matrix"]);
        v.set_issuer(&["starstats-test"]);
        let claims = decode::<serde_json::Value>(&token, &decoding, &v)
            .unwrap()
            .claims;
        assert_eq!(
            claims["sub"], "wing_man",
            "the lowercased handle is the localpart"
        );
        assert_eq!(
            claims["name"], "Wing_Man",
            "the handle as written is the name"
        );
        assert_eq!(signer.user_id("Wing_Man"), "@wing_man:starstats.app");
    }

    #[test]
    fn a_token_for_another_audience_is_not_this_one() {
        let (signer, decoding) = signer();
        let (token, _) = signer.mint("a", Utc::now()).unwrap();
        let mut v = Validation::new(Algorithm::RS256);
        v.set_audience(&["starstats"]);
        assert!(decode::<serde_json::Value>(&token, &decoding, &v).is_err());
    }

    #[test]
    fn a_declaration_counts_only_against_the_current_minimum() {
        let now = Utc::now();
        assert!(!declared_for_chat(None));
        assert!(declared_for_chat(Some((now, CHAT_MIN_AGE))));
        assert!(
            !declared_for_chat(Some((now, CHAT_MIN_AGE - 2))),
            "an older, lower minimum"
        );
    }

    #[tokio::test]
    async fn declaring_again_keeps_the_first_date() {
        let s = MemoryChatAccessStore::new();
        s.declare_age("Wingman", CHAT_MIN_AGE).await.unwrap();
        let first = s.age_declaration("wingman").await.unwrap().unwrap().0;
        s.declare_age("WINGMAN", CHAT_MIN_AGE).await.unwrap();
        assert_eq!(
            s.age_declaration("Wingman").await.unwrap().unwrap().0,
            first
        );
    }

    /// The declaration SQL against the real schema. Skipped without
    /// STARSTATS_TEST_DATABASE_URL.
    #[tokio::test]
    async fn postgres_age_declaration_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres chat access test");
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
        let handle = format!(
            "AgeProbe{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, claimed_handle)
             VALUES (gen_random_uuid(), $1, 'x', $2)",
        )
        .bind(format!("{}@example.com", handle.to_lowercase()))
        .bind(&handle)
        .execute(&pool)
        .await
        .unwrap();
        let s = PostgresChatAccessStore::new(pool.clone());
        assert_eq!(s.age_declaration(&handle).await.unwrap(), None);
        s.declare_age(&handle.to_uppercase(), CHAT_MIN_AGE)
            .await
            .unwrap();
        let (first, min) = s.age_declaration(&handle).await.unwrap().unwrap();
        assert_eq!(min, CHAT_MIN_AGE);
        s.declare_age(&handle, CHAT_MIN_AGE).await.unwrap();
        assert_eq!(s.age_declaration(&handle).await.unwrap().unwrap().0, first);
        sqlx::query("DELETE FROM users WHERE claimed_handle = $1")
            .bind(&handle)
            .execute(&pool)
            .await
            .unwrap();
    }
}
