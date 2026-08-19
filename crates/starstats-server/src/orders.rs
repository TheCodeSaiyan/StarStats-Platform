//! Revolut order tracking — the local mirror of the merchant API order
//! lifecycle. See `migrations/0018_revolut_orders.sql` for the schema
//! and `docs/REVOLUT-INTEGRATION-PLAN.md` for the wider flow.
//!
//! Three concerns live here:
//!  1. The data store (CRUD on `revolut_orders` + insert-or-skip on
//!     `revolut_webhook_events` for redelivery dedup).
//!  2. The state-machine transitions: `pending` → `completed` via the
//!     webhook handler; `pending` → `cancelled`/`failed`/`refunded` via
//!     other event types.
//!  3. The webhook dedup primitive: [`OrderStore::record_webhook_event`]
//!     returns `true` only the first time we see a given
//!     `(revolut_order_id, event_type)` pair, letting the route layer
//!     fence side effects behind that flag.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use uuid::Uuid;

/// One row from `revolut_orders`. Surfaced both to the route layer
/// (the `find_by_revolut_id` lookup hands a snapshot back to the
/// webhook handler) and the supporter mutation path (we read
/// `name_plate_at_checkout` to seed the supporter row's plate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevolutOrderRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub revolut_order_id: Option<String>,
    pub tier_key: String,
    pub amount_minor: i64,
    pub currency: String,
    pub name_plate_at_checkout: Option<String>,
    pub state: OrderState,
    pub checkout_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    Pending,
    Completed,
    Cancelled,
    Failed,
    Refunded,
}

impl OrderState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Refunded => "refunded",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed,
            "refunded" => Self::Refunded,
            _ => return None,
        })
    }
}

/// Fields the route layer supplies when first creating a pending order.
/// `revolut_order_id` and `checkout_url` get stitched in afterwards
/// once Revolut hands them back, via [`OrderStore::attach_revolut_details`].
#[derive(Debug, Clone)]
pub struct NewOrder<'a> {
    pub user_id: Uuid,
    pub tier_key: &'a str,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub name_plate: Option<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum OrderError {
    #[error("order not found")]
    NotFound,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait OrderStore: Send + Sync + 'static {
    /// Create a `pending` row and return its UUIDv7. The id is the
    /// stable handle we pass to Revolut as `merchant_order_ext_ref`,
    /// so it must be generated before we call the merchant API.
    async fn create_pending(&self, new: NewOrder<'_>) -> Result<Uuid, OrderError>;

    /// Stitch Revolut's order id and the hosted checkout URL onto the
    /// pending row we created above. Called once, immediately after
    /// the merchant API returns. Idempotent on `revolut_order_id`
    /// because the column is UNIQUE — replaying this would error.
    async fn attach_revolut_details(
        &self,
        local_id: Uuid,
        revolut_order_id: &str,
        checkout_url: &str,
    ) -> Result<(), OrderError>;

    /// Look up the local order by Revolut's id (the value Revolut sent
    /// back in the create-order response, NOT our local UUID). The
    /// webhook handler is the only caller — it correlates incoming
    /// events to local rows this way.
    async fn find_by_revolut_id(
        &self,
        revolut_order_id: &str,
    ) -> Result<Option<RevolutOrderRow>, OrderError>;

    /// Flip the row's state. Sets `completed_at` to NOW() iff the
    /// target state is `completed`; clears it otherwise (a refunded
    /// order's "completed_at" is no longer meaningful — better surfaced
    /// as null than as a stale timestamp).
    async fn mark_state(&self, revolut_order_id: &str, state: OrderState)
        -> Result<(), OrderError>;

    /// Record a webhook delivery for dedup. Returns `true` iff the
    /// row was newly inserted; `false` means we've already processed
    /// this `(revolut_order_id, event_type)` pair and the caller should
    /// skip side effects. The whole webhook payload is stashed as
    /// JSONB for forensic state — `payload` is opaque to this layer.
    async fn record_webhook_event(
        &self,
        revolut_order_id: &str,
        event_type: &str,
        payload: &JsonValue,
    ) -> Result<bool, OrderError>;
}

// -- Postgres impl ---------------------------------------------------

pub struct PostgresOrderStore {
    pool: PgPool,
}

impl PostgresOrderStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OrderStore for PostgresOrderStore {
    async fn create_pending(&self, new: NewOrder<'_>) -> Result<Uuid, OrderError> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO revolut_orders
                (id, user_id, tier_key, amount_minor, currency, name_plate_at_checkout)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(new.user_id)
        .bind(new.tier_key)
        .bind(new.amount_minor)
        .bind(new.currency)
        .bind(new.name_plate)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    async fn attach_revolut_details(
        &self,
        local_id: Uuid,
        revolut_order_id: &str,
        checkout_url: &str,
    ) -> Result<(), OrderError> {
        let res = sqlx::query(
            "UPDATE revolut_orders
                SET revolut_order_id = $2,
                    checkout_url = $3
              WHERE id = $1",
        )
        .bind(local_id)
        .bind(revolut_order_id)
        .bind(checkout_url)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(OrderError::NotFound);
        }
        Ok(())
    }

    async fn find_by_revolut_id(
        &self,
        revolut_order_id: &str,
    ) -> Result<Option<RevolutOrderRow>, OrderError> {
        let row: Option<(
            Uuid,
            Uuid,
            Option<String>,
            String,
            i64,
            String,
            Option<String>,
            String,
            Option<String>,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT id, user_id, revolut_order_id, tier_key, amount_minor, currency,
                    name_plate_at_checkout, state, checkout_url, created_at, completed_at
               FROM revolut_orders
              WHERE revolut_order_id = $1",
        )
        .bind(revolut_order_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| RevolutOrderRow {
            id: r.0,
            user_id: r.1,
            revolut_order_id: r.2,
            tier_key: r.3,
            amount_minor: r.4,
            currency: r.5,
            name_plate_at_checkout: r.6,
            state: OrderState::parse(&r.7).unwrap_or(OrderState::Pending),
            checkout_url: r.8,
            created_at: r.9,
            completed_at: r.10,
        }))
    }

    async fn mark_state(
        &self,
        revolut_order_id: &str,
        state: OrderState,
    ) -> Result<(), OrderError> {
        let res = sqlx::query(
            "UPDATE revolut_orders
                SET state = $2,
                    completed_at = CASE WHEN $2 = 'completed' THEN NOW() ELSE NULL END
              WHERE revolut_order_id = $1",
        )
        .bind(revolut_order_id)
        .bind(state.as_str())
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(OrderError::NotFound);
        }
        Ok(())
    }

    async fn record_webhook_event(
        &self,
        revolut_order_id: &str,
        event_type: &str,
        payload: &JsonValue,
    ) -> Result<bool, OrderError> {
        let inserted: Option<(String,)> = sqlx::query_as(
            "INSERT INTO revolut_webhook_events
                (revolut_order_id, event_type, payload)
             VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING
             RETURNING revolut_order_id",
        )
        .bind(revolut_order_id)
        .bind(event_type)
        .bind(payload)
        .fetch_optional(&self.pool)
        .await?;
        Ok(inserted.is_some())
    }
}

// -- In-memory test impl ---------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    #[derive(Default)]
    struct State {
        orders: HashMap<Uuid, RevolutOrderRow>,
        // Reverse index: revolut_order_id -> local UUID. Lets
        // find_by_revolut_id avoid a scan.
        by_revolut: HashMap<String, Uuid>,
        seen_events: HashSet<(String, String)>,
    }

    pub struct MemoryOrderStore {
        state: Mutex<State>,
    }

    impl Default for MemoryOrderStore {
        fn default() -> Self {
            Self {
                state: Mutex::new(State::default()),
            }
        }
    }

    #[async_trait]
    impl OrderStore for MemoryOrderStore {
        async fn create_pending(&self, new: NewOrder<'_>) -> Result<Uuid, OrderError> {
            let id = Uuid::now_v7();
            let now = Utc::now();
            self.state
                .lock()
                .expect("orders memstore poisoned")
                .orders
                .insert(
                    id,
                    RevolutOrderRow {
                        id,
                        user_id: new.user_id,
                        revolut_order_id: None,
                        tier_key: new.tier_key.into(),
                        amount_minor: new.amount_minor,
                        currency: new.currency.into(),
                        name_plate_at_checkout: new.name_plate.map(str::to_string),
                        state: OrderState::Pending,
                        checkout_url: None,
                        created_at: now,
                        completed_at: None,
                    },
                );
            Ok(id)
        }

        async fn attach_revolut_details(
            &self,
            local_id: Uuid,
            revolut_order_id: &str,
            checkout_url: &str,
        ) -> Result<(), OrderError> {
            let mut s = self.state.lock().expect("orders memstore poisoned");
            let row = s.orders.get_mut(&local_id).ok_or(OrderError::NotFound)?;
            row.revolut_order_id = Some(revolut_order_id.to_string());
            row.checkout_url = Some(checkout_url.to_string());
            s.by_revolut.insert(revolut_order_id.to_string(), local_id);
            Ok(())
        }

        async fn find_by_revolut_id(
            &self,
            revolut_order_id: &str,
        ) -> Result<Option<RevolutOrderRow>, OrderError> {
            let s = self.state.lock().expect("orders memstore poisoned");
            Ok(s.by_revolut
                .get(revolut_order_id)
                .and_then(|id| s.orders.get(id))
                .cloned())
        }

        async fn mark_state(
            &self,
            revolut_order_id: &str,
            state: OrderState,
        ) -> Result<(), OrderError> {
            let mut s = self.state.lock().expect("orders memstore poisoned");
            let local = *s
                .by_revolut
                .get(revolut_order_id)
                .ok_or(OrderError::NotFound)?;
            let row = s.orders.get_mut(&local).ok_or(OrderError::NotFound)?;
            row.state = state;
            row.completed_at = if state == OrderState::Completed {
                Some(Utc::now())
            } else {
                None
            };
            Ok(())
        }

        async fn record_webhook_event(
            &self,
            revolut_order_id: &str,
            event_type: &str,
            _payload: &JsonValue,
        ) -> Result<bool, OrderError> {
            let mut s = self.state.lock().expect("orders memstore poisoned");
            Ok(s.seen_events
                .insert((revolut_order_id.to_string(), event_type.to_string())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryOrderStore;
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn create_pending_then_attach_makes_findable() {
        let store = MemoryOrderStore::default();
        let user_id = Uuid::now_v7();
        let local = store
            .create_pending(NewOrder {
                user_id,
                tier_key: "standard",
                amount_minor: 500,
                currency: "GBP",
                name_plate: Some("Caelum"),
            })
            .await
            .unwrap();

        // Before attach: not findable by revolut id.
        assert!(store.find_by_revolut_id("rev_abc").await.unwrap().is_none());

        store
            .attach_revolut_details(local, "rev_abc", "https://checkout.example/abc")
            .await
            .unwrap();

        let row = store
            .find_by_revolut_id("rev_abc")
            .await
            .unwrap()
            .expect("present");
        assert_eq!(row.id, local);
        assert_eq!(row.user_id, user_id);
        assert_eq!(row.tier_key, "standard");
        assert_eq!(row.amount_minor, 500);
        assert_eq!(row.currency, "GBP");
        assert_eq!(row.name_plate_at_checkout.as_deref(), Some("Caelum"));
        assert_eq!(row.state, OrderState::Pending);
        assert_eq!(
            row.checkout_url.as_deref(),
            Some("https://checkout.example/abc")
        );
    }

    #[tokio::test]
    async fn mark_state_completed_sets_completed_at() {
        let store = MemoryOrderStore::default();
        let user_id = Uuid::now_v7();
        let local = store
            .create_pending(NewOrder {
                user_id,
                tier_key: "coffee",
                amount_minor: 200,
                currency: "GBP",
                name_plate: None,
            })
            .await
            .unwrap();
        store
            .attach_revolut_details(local, "rev_xyz", "https://checkout.example/xyz")
            .await
            .unwrap();

        store
            .mark_state("rev_xyz", OrderState::Completed)
            .await
            .unwrap();
        let row = store.find_by_revolut_id("rev_xyz").await.unwrap().unwrap();
        assert_eq!(row.state, OrderState::Completed);
        assert!(row.completed_at.is_some());

        // Refund clears completed_at.
        store
            .mark_state("rev_xyz", OrderState::Refunded)
            .await
            .unwrap();
        let row = store.find_by_revolut_id("rev_xyz").await.unwrap().unwrap();
        assert_eq!(row.state, OrderState::Refunded);
        assert!(row.completed_at.is_none());
    }

    #[tokio::test]
    async fn record_webhook_event_dedups_per_pair() {
        let store = MemoryOrderStore::default();
        let payload = json!({"event": "ORDER_COMPLETED"});

        let first = store
            .record_webhook_event("rev_abc", "ORDER_COMPLETED", &payload)
            .await
            .unwrap();
        assert!(first, "first delivery should record");

        let second = store
            .record_webhook_event("rev_abc", "ORDER_COMPLETED", &payload)
            .await
            .unwrap();
        assert!(!second, "redelivery should be deduped");

        // Different event_type for same order is a fresh row.
        let third = store
            .record_webhook_event("rev_abc", "ORDER_FAILED", &payload)
            .await
            .unwrap();
        assert!(third, "different event type should record");
    }

    #[tokio::test]
    async fn find_by_revolut_id_returns_none_for_unknown() {
        let store = MemoryOrderStore::default();
        assert!(store.find_by_revolut_id("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn mark_state_unknown_id_returns_not_found() {
        let store = MemoryOrderStore::default();
        let err = store
            .mark_state("nope", OrderState::Completed)
            .await
            .unwrap_err();
        assert!(matches!(err, OrderError::NotFound));
    }
}
