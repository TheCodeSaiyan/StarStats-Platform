//! Supporter (donate) status data layer.
//!
//! See `docs/REVOLUT-INTEGRATION-PLAN.md` for the full lifecycle.
//! This module owns the read side and one mutation (set name_plate);
//! payment-flow mutations (state transitions on webhook) land in the
//! Wave 9 follow-up that wires up Revolut.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Hard cap on the display name string. The design spec is 28 chars;
/// we enforce here so a future API change can't accidentally relax it
/// without an explicit override. Read-side endpoint doesn't reference
/// this; the cap kicks in on the PUT endpoint that ships in the
/// Wave 9 follow-up (see `docs/REVOLUT-INTEGRATION-PLAN.md`).
#[allow(dead_code)]
pub const NAME_PLATE_MAX_CHARS: usize = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupporterState {
    None,
    Active,
    Lapsed,
}

impl SupporterState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Active => "active",
            Self::Lapsed => "lapsed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Self::None,
            "active" => Self::Active,
            "lapsed" => Self::Lapsed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SupporterStatus {
    /// The owning user — populated for symmetry with other rows but
    /// not surfaced by the current read DTO (the caller's identity is
    /// already in the bearer token).
    #[allow(dead_code)]
    pub user_id: Uuid,
    pub state: SupporterState,
    pub name_plate: Option<String>,
    pub became_supporter_at: Option<DateTime<Utc>>,
    pub last_payment_at: Option<DateTime<Utc>>,
    pub grace_until: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    /// Last write timestamp; surfaced by the eventual webhook handler
    /// for stale-row checks, not by the read DTO.
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
    /// `tier_key` of the most-recent completed Revolut order. Derived
    /// at read time from `revolut_orders` because the schema header
    /// for `supporter_status` deliberately keeps the dollar amount
    /// off this table (tier rename safety + denorm-drift avoidance).
    /// `None` when the user has never had a completed order — even
    /// if `state` is `active` for a state-only path (which today
    /// can't actually happen since the only writer is the webhook).
    /// Powers the tier-specific styling on the supporter chip.
    pub current_tier_key: Option<String>,
}

impl SupporterStatus {
    /// Default surface for users with no row yet. Saves the read path
    /// from a special "no row" branch — every authenticated user can
    /// at least say "none + no plate".
    pub fn empty(user_id: Uuid) -> Self {
        Self {
            user_id,
            state: SupporterState::None,
            name_plate: None,
            became_supporter_at: None,
            last_payment_at: None,
            grace_until: None,
            cancelled_at: None,
            updated_at: Utc::now(),
            current_tier_key: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SupporterError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait SupporterStore: Send + Sync + 'static {
    /// Returns the row for `user_id` or [`SupporterStatus::empty`] if
    /// no row exists yet. Never returns `None`; the caller's mental
    /// model is "every user has a status, the default is none".
    async fn get(&self, user_id: Uuid) -> Result<SupporterStatus, SupporterError>;

    /// Public-safe lookup keyed by RSI handle, for rendering the
    /// supporter chip on public + friend profile views.
    ///
    /// Returns `Ok(None)` when ANY of the following hold:
    ///   - no `users` row matches the handle (case-insensitive)
    ///   - the user has no `supporter_status` row
    ///   - the row's state is `none`
    ///
    /// Returns `Ok(Some(_))` only for `active` or `lapsed` states —
    /// callers don't need to filter again. Wraps `SupporterStatus`
    /// so it carries `current_tier_key` (tier styling) +
    /// `name_plate` (display string) without surfacing fields like
    /// `grace_until` / `last_payment_at` to public callers (the
    /// route layer's `PublicSupporterInfo` projection drops them).
    ///
    /// The handle-keyed query JOINs `users` + `supporter_status` +
    /// the most-recent completed `revolut_orders` row in one
    /// round-trip — same lateral-join shape as `get`, just with a
    /// handle lookup tacked on.
    async fn get_by_handle_public(
        &self,
        handle: &str,
    ) -> Result<Option<SupporterStatus>, SupporterError>;

    /// Bulk variant of [`Self::get_by_handle_public`] for the
    /// `/v1/discover/profiles` listing. Returns a map keyed by the
    /// LOWERCASED handle so callers can `HashMap::get` against
    /// `handle.to_ascii_lowercase()` without re-allocating per row.
    ///
    /// Only `active` + `lapsed` supporters are surfaced — non-supporter
    /// users simply have no entry in the returned map.
    ///
    /// Critical: this MUST be a single SQL round-trip with
    /// `WHERE LOWER(claimed_handle) = ANY($1)`. Naïve N round-trips
    /// (one per profile) would scale linearly with `DEFAULT_LIMIT`
    /// (50) — death by latency for what's already a SpiceDB +
    /// users-table query.
    ///
    /// Empty input returns an empty map without a DB round-trip.
    async fn get_many_public_by_handle(
        &self,
        handles: &[String],
    ) -> Result<std::collections::HashMap<String, SupporterStatus>, SupporterError>;

    /// Flip the user's state to `active` and record a payment. The
    /// webhook handler calls this when an `ORDER_COMPLETED` event
    /// lands. Idempotent: replaying the same payment is a no-op
    /// because the `revolut_webhook_events` PK fences duplicate
    /// webhook deliveries before this method is reached.
    ///
    /// `name_plate`, when `Some`, sets/replaces the existing plate.
    /// `None` leaves any existing plate untouched (don't overwrite a
    /// plate the user set later via the edit endpoint with a stale
    /// snapshot from an older order).
    async fn mark_payment_received(
        &self,
        user_id: Uuid,
        name_plate: Option<&str>,
        coverage_until: DateTime<Utc>,
    ) -> Result<(), SupporterError>;
}

pub struct PostgresSupporterStore {
    pool: PgPool,
}

impl PostgresSupporterStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SupporterStore for PostgresSupporterStore {
    async fn get(&self, user_id: Uuid) -> Result<SupporterStatus, SupporterError> {
        // Single-row read joined to the user's most-recent completed
        // revolut_orders row for the `current_tier_key` derivation.
        // `LEFT JOIN LATERAL ... ON true` keeps the supporter_status
        // row even when no completed orders exist (state could be
        // `active` from a future state-only path; today every active
        // row has at least one order but we don't want the join to
        // suppress the supporter row if a backfill case ever exists).
        let row: Option<(
            String,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT s.state, s.name_plate,
                    s.became_supporter_at, s.last_payment_at,
                    s.grace_until, s.cancelled_at, s.updated_at,
                    o.tier_key
             FROM supporter_status s
             LEFT JOIN LATERAL (
                 SELECT tier_key
                 FROM revolut_orders
                 WHERE user_id = s.user_id AND state = 'completed'
                 ORDER BY completed_at DESC NULLS LAST, created_at DESC
                 LIMIT 1
             ) o ON true
             WHERE s.user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(match row {
            None => SupporterStatus::empty(user_id),
            Some((state, name_plate, became, last_pay, grace, cancelled, updated, tier_key)) => {
                SupporterStatus {
                    user_id,
                    state: SupporterState::parse(&state).unwrap_or(SupporterState::None),
                    name_plate,
                    became_supporter_at: became,
                    last_payment_at: last_pay,
                    grace_until: grace,
                    cancelled_at: cancelled,
                    updated_at: updated,
                    current_tier_key: tier_key,
                }
            }
        })
    }

    async fn get_by_handle_public(
        &self,
        handle: &str,
    ) -> Result<Option<SupporterStatus>, SupporterError> {
        // Public-safe handle-keyed lookup. Inner-joins users +
        // supporter_status so we get a row iff BOTH exist; the
        // `state IN ('active','lapsed')` predicate excludes `none`
        // (no chip should render for those users). Lateral join for
        // current_tier_key mirrors `get`.
        let row: Option<(
            Uuid,
            String,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT s.user_id, s.state, s.name_plate,
                    s.became_supporter_at, s.last_payment_at,
                    s.grace_until, s.cancelled_at, s.updated_at,
                    o.tier_key
             FROM users u
             INNER JOIN supporter_status s ON s.user_id = u.id
             LEFT JOIN LATERAL (
                 SELECT tier_key
                 FROM revolut_orders
                 WHERE user_id = s.user_id AND state = 'completed'
                 ORDER BY completed_at DESC NULLS LAST, created_at DESC
                 LIMIT 1
             ) o ON true
             WHERE LOWER(u.claimed_handle) = LOWER($1)
               AND s.state IN ('active', 'lapsed')",
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(
                user_id,
                state,
                name_plate,
                became,
                last_pay,
                grace,
                cancelled,
                updated,
                tier_key,
            )| {
                SupporterStatus {
                    user_id,
                    state: SupporterState::parse(&state).unwrap_or(SupporterState::None),
                    name_plate,
                    became_supporter_at: became,
                    last_payment_at: last_pay,
                    grace_until: grace,
                    cancelled_at: cancelled,
                    updated_at: updated,
                    current_tier_key: tier_key,
                }
            },
        ))
    }

    async fn get_many_public_by_handle(
        &self,
        handles: &[String],
    ) -> Result<std::collections::HashMap<String, SupporterStatus>, SupporterError> {
        use std::collections::HashMap;
        if handles.is_empty() {
            return Ok(HashMap::new());
        }
        // Normalise to lowercase once on the wire so the SQL `= ANY($1)`
        // matches against `LOWER(claimed_handle)`. Mirrors the same
        // pattern used by `discover_routes::list_public_profiles_filtered`.
        let normalized: Vec<String> = handles.iter().map(|h| h.to_ascii_lowercase()).collect();

        // Same JOIN shape as `get_by_handle_public`, just keyed by an
        // array. `DISTINCT ON (s.user_id)` collapses the LATERAL
        // tier-lookup result to one row per user (defensive — without
        // DISTINCT a future change that joined a 1-to-many side
        // would silently inflate the response). The supporter +
        // user pair are inherently 1-to-1 since `user_id` is the
        // supporter_status PK, so DISTINCT is currently a no-op but
        // it cheaply hardens the bulk path against future drift.
        let rows: Vec<(
            String,
            Uuid,
            String,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT DISTINCT ON (s.user_id)
                    LOWER(u.claimed_handle) AS handle_key,
                    s.user_id, s.state, s.name_plate,
                    s.became_supporter_at, s.last_payment_at,
                    s.grace_until, s.cancelled_at, s.updated_at,
                    o.tier_key
             FROM users u
             INNER JOIN supporter_status s ON s.user_id = u.id
             LEFT JOIN LATERAL (
                 SELECT tier_key
                 FROM revolut_orders
                 WHERE user_id = s.user_id AND state = 'completed'
                 ORDER BY completed_at DESC NULLS LAST, created_at DESC
                 LIMIT 1
             ) o ON true
             WHERE LOWER(u.claimed_handle) = ANY($1)
               AND s.state IN ('active', 'lapsed')",
        )
        .bind(&normalized)
        .fetch_all(&self.pool)
        .await?;

        let mut map = HashMap::with_capacity(rows.len());
        for (
            handle_key,
            user_id,
            state,
            name_plate,
            became,
            last_pay,
            grace,
            cancelled,
            updated,
            tier_key,
        ) in rows
        {
            map.insert(
                handle_key,
                SupporterStatus {
                    user_id,
                    state: SupporterState::parse(&state).unwrap_or(SupporterState::None),
                    name_plate,
                    became_supporter_at: became,
                    last_payment_at: last_pay,
                    grace_until: grace,
                    cancelled_at: cancelled,
                    updated_at: updated,
                    current_tier_key: tier_key,
                },
            );
        }
        Ok(map)
    }

    async fn mark_payment_received(
        &self,
        user_id: Uuid,
        name_plate: Option<&str>,
        coverage_until: DateTime<Utc>,
    ) -> Result<(), SupporterError> {
        // Single UPSERT: insert the row if missing, otherwise advance
        // its state. `became_supporter_at` is set on first payment
        // only (COALESCE keeps the original value on subsequent
        // payments). `name_plate` only overwrites when the caller
        // supplied one, so a later user-driven plate edit isn't
        // clobbered by a future payment that didn't carry a plate.
        sqlx::query(
            "INSERT INTO supporter_status
                (user_id, state, name_plate, became_supporter_at,
                 last_payment_at, grace_until, cancelled_at, updated_at)
             VALUES ($1, 'active', $2, NOW(), NOW(), $3, NULL, NOW())
             ON CONFLICT (user_id) DO UPDATE SET
                state = 'active',
                name_plate = COALESCE($2, supporter_status.name_plate),
                became_supporter_at =
                    COALESCE(supporter_status.became_supporter_at, NOW()),
                last_payment_at = NOW(),
                grace_until = $3,
                cancelled_at = NULL,
                updated_at = NOW()",
        )
        .bind(user_id)
        .bind(name_plate)
        .bind(coverage_until)
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
    pub struct MemorySupporterStore {
        rows: Mutex<HashMap<Uuid, SupporterStatus>>,
        // Handle -> user_id mapping for the public-handle lookup
        // path. Tests that exercise `get_by_handle_public` seed both
        // a row (via `seed`) AND a handle binding (via `bind_handle`);
        // tests that only exercise `get` just call `seed`.
        handles: Mutex<HashMap<String, Uuid>>,
    }

    impl MemorySupporterStore {
        pub fn seed(&self, status: SupporterStatus) {
            self.rows
                .lock()
                .expect("supporter memstore poisoned")
                .insert(status.user_id, status);
        }

        /// Bind a handle (case-insensitive) to a user_id so
        /// `get_by_handle_public` resolves. Mirrors what users +
        /// supporter_status joined together would surface.
        pub fn bind_handle(&self, handle: &str, user_id: Uuid) {
            self.handles
                .lock()
                .expect("supporter memstore poisoned")
                .insert(handle.to_lowercase(), user_id);
        }
    }

    #[async_trait]
    impl SupporterStore for MemorySupporterStore {
        async fn get(&self, user_id: Uuid) -> Result<SupporterStatus, SupporterError> {
            let rows = self.rows.lock().expect("supporter memstore poisoned");
            Ok(rows
                .get(&user_id)
                .cloned()
                .unwrap_or_else(|| SupporterStatus::empty(user_id)))
        }

        async fn get_by_handle_public(
            &self,
            handle: &str,
        ) -> Result<Option<SupporterStatus>, SupporterError> {
            let user_id = {
                let handles = self.handles.lock().expect("supporter memstore poisoned");
                match handles.get(&handle.to_lowercase()) {
                    Some(uid) => *uid,
                    None => return Ok(None),
                }
            };
            let rows = self.rows.lock().expect("supporter memstore poisoned");
            let status = rows.get(&user_id).cloned();
            // Mirror the Postgres state filter: only `active` /
            // `lapsed` resolve.
            Ok(status
                .filter(|s| matches!(s.state, SupporterState::Active | SupporterState::Lapsed)))
        }

        async fn get_many_public_by_handle(
            &self,
            handles: &[String],
        ) -> Result<HashMap<String, SupporterStatus>, SupporterError> {
            if handles.is_empty() {
                return Ok(HashMap::new());
            }
            let bindings = self.handles.lock().expect("supporter memstore poisoned");
            let rows = self.rows.lock().expect("supporter memstore poisoned");
            let mut out = HashMap::new();
            for h in handles {
                let key = h.to_ascii_lowercase();
                let Some(uid) = bindings.get(&key).copied() else {
                    continue;
                };
                if let Some(status) = rows.get(&uid).cloned() {
                    if matches!(
                        status.state,
                        SupporterState::Active | SupporterState::Lapsed
                    ) {
                        out.insert(key, status);
                    }
                }
            }
            Ok(out)
        }

        async fn mark_payment_received(
            &self,
            user_id: Uuid,
            name_plate: Option<&str>,
            coverage_until: DateTime<Utc>,
        ) -> Result<(), SupporterError> {
            let mut rows = self.rows.lock().expect("supporter memstore poisoned");
            let now = Utc::now();
            let entry = rows.entry(user_id).or_insert_with(|| SupporterStatus {
                user_id,
                state: SupporterState::None,
                name_plate: None,
                became_supporter_at: None,
                last_payment_at: None,
                grace_until: None,
                cancelled_at: None,
                updated_at: now,
                current_tier_key: None,
            });
            entry.state = SupporterState::Active;
            if let Some(plate) = name_plate {
                entry.name_plate = Some(plate.to_string());
            }
            if entry.became_supporter_at.is_none() {
                entry.became_supporter_at = Some(now);
            }
            entry.last_payment_at = Some(now);
            entry.grace_until = Some(coverage_until);
            entry.cancelled_at = None;
            entry.updated_at = now;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemorySupporterStore;
    use super::*;

    #[tokio::test]
    async fn get_returns_none_state_for_unseeded_user() {
        let store = MemorySupporterStore::default();
        let user_id = Uuid::now_v7();
        let s = store.get(user_id).await.expect("get");
        assert_eq!(s.state, SupporterState::None);
        assert!(s.name_plate.is_none());
    }

    #[tokio::test]
    async fn mark_payment_received_creates_active_row() {
        let store = MemorySupporterStore::default();
        let user_id = Uuid::now_v7();
        let coverage = Utc::now() + chrono::Duration::days(30);
        store
            .mark_payment_received(user_id, Some("Caelum"), coverage)
            .await
            .expect("mark");
        let s = store.get(user_id).await.unwrap();
        assert_eq!(s.state, SupporterState::Active);
        assert_eq!(s.name_plate.as_deref(), Some("Caelum"));
        assert!(s.became_supporter_at.is_some());
        assert!(s.last_payment_at.is_some());
        assert_eq!(s.grace_until, Some(coverage));
    }

    #[tokio::test]
    async fn mark_payment_received_preserves_existing_plate_when_none_passed() {
        let store = MemorySupporterStore::default();
        let user_id = Uuid::now_v7();
        let coverage = Utc::now() + chrono::Duration::days(30);
        store
            .mark_payment_received(user_id, Some("FirstPlate"), coverage)
            .await
            .unwrap();
        // Second payment without plate: keep the first plate.
        store
            .mark_payment_received(user_id, None, coverage + chrono::Duration::days(30))
            .await
            .unwrap();
        let s = store.get(user_id).await.unwrap();
        assert_eq!(s.name_plate.as_deref(), Some("FirstPlate"));
    }

    #[tokio::test]
    async fn get_returns_seeded_row() {
        let store = MemorySupporterStore::default();
        let user_id = Uuid::now_v7();
        store.seed(SupporterStatus {
            user_id,
            state: SupporterState::Active,
            name_plate: Some("Caelum".into()),
            became_supporter_at: Some(Utc::now()),
            last_payment_at: Some(Utc::now()),
            grace_until: None,
            cancelled_at: None,
            updated_at: Utc::now(),
            current_tier_key: Some("coffee".into()),
        });
        let s = store.get(user_id).await.unwrap();
        assert_eq!(s.state, SupporterState::Active);
        assert_eq!(s.name_plate.as_deref(), Some("Caelum"));
        assert_eq!(s.current_tier_key.as_deref(), Some("coffee"));
    }

    #[tokio::test]
    async fn get_by_handle_public_returns_none_for_unknown_handle() {
        let store = MemorySupporterStore::default();
        let result = store.get_by_handle_public("Nobody").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_by_handle_public_returns_none_for_unbound_handle() {
        // Row exists but handle binding is missing — handle-keyed
        // lookup must miss even though user_id-keyed lookup would
        // find it. Mirrors the Postgres JOIN failing on
        // `users.claimed_handle`.
        let store = MemorySupporterStore::default();
        let user_id = Uuid::now_v7();
        store.seed(SupporterStatus {
            user_id,
            state: SupporterState::Active,
            name_plate: Some("Caelum".into()),
            became_supporter_at: Some(Utc::now()),
            last_payment_at: Some(Utc::now()),
            grace_until: None,
            cancelled_at: None,
            updated_at: Utc::now(),
            current_tier_key: Some("coffee".into()),
        });
        let result = store.get_by_handle_public("Caelum").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_by_handle_public_returns_active_row_when_bound() {
        let store = MemorySupporterStore::default();
        let user_id = Uuid::now_v7();
        store.seed(SupporterStatus {
            user_id,
            state: SupporterState::Active,
            name_plate: Some("Caelum".into()),
            became_supporter_at: Some(Utc::now()),
            last_payment_at: Some(Utc::now()),
            grace_until: None,
            cancelled_at: None,
            updated_at: Utc::now(),
            current_tier_key: Some("generous".into()),
        });
        store.bind_handle("Caelum", user_id);
        // Lookup is case-insensitive — mirrors LOWER() in SQL.
        let result = store
            .get_by_handle_public("caelum")
            .await
            .unwrap()
            .expect("supporter present");
        assert_eq!(result.state, SupporterState::Active);
        assert_eq!(result.current_tier_key.as_deref(), Some("generous"));
        assert_eq!(result.name_plate.as_deref(), Some("Caelum"));
    }

    #[tokio::test]
    async fn get_many_public_by_handle_returns_empty_for_empty_input() {
        let store = MemorySupporterStore::default();
        let result = store
            .get_many_public_by_handle(&[])
            .await
            .expect("get_many");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn get_many_public_by_handle_skips_unknown_and_unbound_handles() {
        let store = MemorySupporterStore::default();
        let known_id = Uuid::now_v7();
        store.seed(SupporterStatus {
            user_id: known_id,
            state: SupporterState::Active,
            name_plate: Some("Caelum".into()),
            became_supporter_at: Some(Utc::now()),
            last_payment_at: Some(Utc::now()),
            grace_until: None,
            cancelled_at: None,
            updated_at: Utc::now(),
            current_tier_key: Some("standard".into()),
        });
        store.bind_handle("Caelum", known_id);
        // Throw in another seeded row with no handle binding to make
        // sure we don't fall through to user_id-keyed iteration.
        let orphan_id = Uuid::now_v7();
        store.seed(SupporterStatus {
            user_id: orphan_id,
            state: SupporterState::Active,
            name_plate: None,
            became_supporter_at: Some(Utc::now()),
            last_payment_at: Some(Utc::now()),
            grace_until: None,
            cancelled_at: None,
            updated_at: Utc::now(),
            current_tier_key: Some("coffee".into()),
        });

        let result = store
            .get_many_public_by_handle(&[
                "caelum".to_string(),
                "Stranger".to_string(),
                "OrphanWithoutHandle".to_string(),
            ])
            .await
            .expect("get_many");
        assert_eq!(result.len(), 1);
        let row = result.get("caelum").expect("caelum present");
        assert_eq!(row.current_tier_key.as_deref(), Some("standard"));
        assert!(!result.contains_key("stranger"));
    }

    #[tokio::test]
    async fn get_many_public_by_handle_filters_none_state() {
        let store = MemorySupporterStore::default();
        let quiet_id = Uuid::now_v7();
        store.seed(SupporterStatus {
            user_id: quiet_id,
            state: SupporterState::None,
            name_plate: None,
            became_supporter_at: None,
            last_payment_at: None,
            grace_until: None,
            cancelled_at: None,
            updated_at: Utc::now(),
            current_tier_key: None,
        });
        store.bind_handle("Quiet", quiet_id);
        let result = store
            .get_many_public_by_handle(&["Quiet".to_string()])
            .await
            .expect("get_many");
        assert!(result.is_empty(), "none-state users must not surface");
    }

    #[tokio::test]
    async fn get_by_handle_public_filters_none_state() {
        // `state = none` users should NOT surface in the public
        // lookup even when bound — the chip render path expects
        // either active or lapsed.
        let store = MemorySupporterStore::default();
        let user_id = Uuid::now_v7();
        store.seed(SupporterStatus {
            user_id,
            state: SupporterState::None,
            name_plate: None,
            became_supporter_at: None,
            last_payment_at: None,
            grace_until: None,
            cancelled_at: None,
            updated_at: Utc::now(),
            current_tier_key: None,
        });
        store.bind_handle("Quiet", user_id);
        let result = store.get_by_handle_public("Quiet").await.unwrap();
        assert!(result.is_none(), "state=none must not surface publicly");
    }
}
