//! HTTP handlers for the donate flow.
//!
//! Two endpoints:
//!   - `POST /v1/donate/checkout` — bearer-auth'd. Creates a local
//!     pending order, calls Revolut to mint a hosted checkout URL, and
//!     returns that URL to the client. The client redirects the user
//!     there.
//!   - `POST /v1/webhooks/revolut` — unauthenticated; trust is keyed
//!     to the HMAC-SHA256 signature on `Revolut-Signature` (see
//!     `revolut::verify_webhook_signature`). On a verified
//!     `ORDER_COMPLETED` the handler flips the supporter row to
//!     `active` and the order row to `completed`. Webhook redeliveries
//!     are deduped by the `revolut_webhook_events` PK.
//!
//! When `RevolutConfig` is absent (no env vars set), both routes
//! return `503 not_configured`. We still mount them so the OpenAPI
//! surface is stable; this is the same posture as the magic-link / RSI
//! routes when their backing service is offline.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::config::RevolutConfig;
use crate::orders::{NewOrder, OrderState, OrderStore, PostgresOrderStore};
use crate::revolut::{
    verify_webhook_signature, CreateOrderRequest, RevolutClient, RevolutError, SignatureError,
    WebhookEvent,
};
use crate::supporters::{PostgresSupporterStore, SupporterStore};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::post,
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Donation tier table. The amounts are denominated in minor units
/// (pence) under `GBP`. Tiers are intentionally hard-coded here
/// rather than stored in the DB: we need the route layer to be able
/// to reject an unknown tier_key on POST without a round-trip, and
/// the set of tiers changes about as often as a price-list edit on
/// the marketing page (i.e. rarely, via PR).
///
/// To add a tier: append a row, run `cargo test`, redeploy. To rename
/// a tier: don't — historical orders carry the old key and a renamed
/// tier breaks their reporting. Add a new key instead.
pub const TIERS: &[Tier] = &[
    Tier {
        key: "coffee",
        amount_minor: 300,
        currency: "GBP",
        label: "Coffee",
        description: "A one-off thank-you, the cost of a flat white.",
    },
    Tier {
        key: "standard",
        amount_minor: 500,
        currency: "GBP",
        label: "Standard supporter",
        description: "Keeps the lights on for one month of hosting + a name plate.",
    },
    Tier {
        key: "generous",
        amount_minor: 1500,
        currency: "GBP",
        label: "Generous supporter",
        description: "Three months of hosting plus a louder thank-you on the supporters list.",
    },
];

#[derive(Debug, Clone, Copy)]
pub struct Tier {
    pub key: &'static str,
    pub amount_minor: i64,
    pub currency: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

/// How long a single payment buys supporter status. We don't model
/// real subscriptions yet — every successful order extends the grace
/// window by this much from completion. `Standard` is one month, but
/// every tier (including `coffee`) currently uses the same window so
/// the "active" badge sticks around for at least a few weeks after a
/// one-off donation. Tweak per-tier later if the billing model evolves.
const COVERAGE_DAYS: i64 = 30;

/// Hard cap on `name_plate` at the API layer. Mirrors
/// [`crate::supporters::NAME_PLATE_MAX_CHARS`] (28). Duplicated here
/// rather than imported because the validation is a route concern —
/// the store doesn't get to see the rejection path either way.
const NAME_PLATE_MAX_CHARS: usize = 28;

/// Combined router state. We hand-wrap the three concerns (orders,
/// supporters, Revolut HTTP client) into a single `State` value so
/// the handlers extract one thing instead of fighting Axum's
/// `FromRequestParts` rules across multiple `State<_>` extractors.
#[derive(Clone)]
pub struct DonateState {
    pub orders: Arc<PostgresOrderStore>,
    pub supporters: Arc<PostgresSupporterStore>,
    /// `None` when `REVOLUT_API_KEY` is absent — handlers short-circuit
    /// to 503. `Some` carries the live HTTP client.
    pub revolut: Option<Arc<RevolutClient>>,
    /// Webhook signing secret, mirrored from [`RevolutConfig`]. `None`
    /// means we still accept the route mount but reject every webhook
    /// as unverified. We keep the boxed-bytes shape so a leak via
    /// debug-print doesn't show the full key.
    pub webhook_secret: Option<Vec<u8>>,
    /// Where Revolut redirects after checkout. `None` falls back to
    /// the merchant default in the dashboard.
    pub return_url: Option<String>,
}

pub fn routes(state: DonateState) -> Router {
    Router::new()
        .route("/v1/donate/tiers", axum::routing::get(list_tiers))
        .route("/v1/donate/checkout", post(checkout))
        .route("/v1/webhooks/revolut", post(webhook))
        .with_state(state)
}

fn err(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

fn err_detail(status: StatusCode, code: &'static str, detail: String) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: Some(detail),
        }),
    )
        .into_response()
}

// -- DTOs -----------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TierDto {
    pub key: String,
    pub amount_minor: i64,
    pub currency: String,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TierListResponse {
    pub tiers: Vec<TierDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckoutRequest {
    /// Must match one of the keys in [`TIERS`].
    pub tier_key: String,
    /// Optional 28-char display name to lock in at payment time. May
    /// be null/empty if the user wants to set their plate via the
    /// edit endpoint later.
    #[serde(default)]
    pub name_plate: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CheckoutResponse {
    pub order_id: String,
    pub checkout_url: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WebhookAck {
    pub status: String,
}

// -- Handlers --------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/donate/tiers",
    tag = "donate",
    operation_id = "donate_list_tiers",
    responses(
        (status = 200, description = "Static tier list", body = TierListResponse),
    )
)]
pub async fn list_tiers() -> Response {
    let body = TierListResponse {
        tiers: TIERS
            .iter()
            .map(|t| TierDto {
                key: t.key.into(),
                amount_minor: t.amount_minor,
                currency: t.currency.into(),
                label: t.label.into(),
                description: t.description.into(),
            })
            .collect(),
    };
    (StatusCode::OK, Json(body)).into_response()
}

#[utoipa::path(
    post,
    path = "/v1/donate/checkout",
    tag = "donate",
    operation_id = "donate_checkout",
    request_body = CheckoutRequest,
    responses(
        (status = 200, description = "Order created; client should redirect to checkout_url", body = CheckoutResponse),
        (status = 400, description = "Validation failure", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 502, description = "Upstream Revolut error", body = ApiErrorBody),
        (status = 503, description = "Revolut not configured", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn checkout(
    State(state): State<DonateState>,
    auth: AuthenticatedUser,
    Json(req): Json<CheckoutRequest>,
) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return err(StatusCode::UNAUTHORIZED, "invalid_subject"),
    };

    let Some(client) = state.revolut.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "not_configured");
    };

    let Some(tier) = TIERS.iter().find(|t| t.key == req.tier_key) else {
        return err(StatusCode::BAD_REQUEST, "unknown_tier");
    };

    // Plate validation. Empty string == "no plate"; we coerce to None
    // so the store doesn't end up with a nonsensical "" row.
    let name_plate = req
        .name_plate
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(plate) = name_plate {
        if plate.chars().count() > NAME_PLATE_MAX_CHARS {
            return err(StatusCode::BAD_REQUEST, "name_plate_too_long");
        }
    }

    // 1. Land the local row first. If Revolut errors after this we
    //    leave a `pending` row behind — that's fine; a daily reaper
    //    can sweep stale pending rows older than ~24h.
    let local_id = match state
        .orders
        .create_pending(NewOrder {
            user_id,
            tier_key: tier.key,
            amount_minor: tier.amount_minor,
            currency: tier.currency,
            name_plate,
        })
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "create_pending failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "order_create_failed");
        }
    };

    let local_id_str = local_id.to_string();
    let req_body = CreateOrderRequest {
        amount: tier.amount_minor,
        currency: tier.currency,
        redirect_url: state.return_url.as_deref(),
        merchant_order_ext_ref: Some(&local_id_str),
        description: Some(tier.label),
    };

    let resp = match client.create_order(&req_body).await {
        Ok(r) => r,
        Err(RevolutError::Status { status, body }) => {
            tracing::warn!(
                status,
                body = %body,
                "Revolut create_order returned non-2xx"
            );
            return err_detail(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                format!("Revolut returned {status}"),
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "Revolut create_order transport error");
            return err(StatusCode::BAD_GATEWAY, "upstream_error");
        }
    };

    if let Err(e) = state
        .orders
        .attach_revolut_details(local_id, &resp.id, &resp.checkout_url)
        .await
    {
        tracing::error!(error = %e, "attach_revolut_details failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "order_persist_failed");
    }

    (
        StatusCode::OK,
        Json(CheckoutResponse {
            order_id: local_id_str,
            checkout_url: resp.checkout_url,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/webhooks/revolut",
    tag = "donate",
    operation_id = "donate_webhook",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Event accepted (or deduped)", body = WebhookAck),
        (status = 400, description = "Body could not be parsed", body = ApiErrorBody),
        (status = 401, description = "Signature verification failed", body = ApiErrorBody),
        (status = 503, description = "Revolut not configured", body = ApiErrorBody),
    )
)]
pub async fn webhook(
    State(state): State<DonateState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(secret) = state.webhook_secret.as_deref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "not_configured");
    };

    let timestamp = headers
        .get("Revolut-Request-Timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let signature = headers
        .get("Revolut-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Err(e) = verify_webhook_signature(secret, timestamp, signature, &body, Utc::now()) {
        tracing::warn!(error = %e, "webhook signature rejected");
        return match e {
            SignatureError::HeaderMissing | SignatureError::TimestampMissing => {
                err(StatusCode::BAD_REQUEST, "missing_signature_header")
            }
            SignatureError::TimestampOutOfRange => {
                err(StatusCode::UNAUTHORIZED, "timestamp_out_of_range")
            }
            SignatureError::NoSupportedScheme | SignatureError::Mismatch => {
                err(StatusCode::UNAUTHORIZED, "signature_invalid")
            }
        };
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "webhook body parse failed");
            return err(StatusCode::BAD_REQUEST, "invalid_body");
        }
    };
    let event: WebhookEvent = match serde_json::from_value(payload.clone()) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "webhook body missing required fields");
            return err(StatusCode::BAD_REQUEST, "invalid_event");
        }
    };

    // Dedup by (revolut_order_id, event). False here means we've
    // already processed this exact delivery — return 200 silently so
    // Revolut stops retrying.
    let inserted = match state
        .orders
        .record_webhook_event(&event.order_id, &event.event, &payload)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "record_webhook_event failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "webhook_persist_failed");
        }
    };
    if !inserted {
        return (
            StatusCode::OK,
            Json(WebhookAck {
                status: "duplicate".into(),
            }),
        )
            .into_response();
    }

    // Resolve the local row. If we don't recognise the order we
    // accept the event (so Revolut stops redelivering) but log loud —
    // it's a data-integrity miss, not a transient one.
    let row = match state.orders.find_by_revolut_id(&event.order_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(order_id = %event.order_id, "webhook for unknown order");
            return (
                StatusCode::OK,
                Json(WebhookAck {
                    status: "ignored".into(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "find_by_revolut_id failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "webhook_lookup_failed");
        }
    };

    // Map Revolut's event slug onto our internal state machine. The
    // exact set Revolut emits today: ORDER_COMPLETED, ORDER_AUTHORISED,
    // ORDER_FAILED, ORDER_CANCELLED. We only act on the terminal ones;
    // ORDER_AUTHORISED is informational because for instant-capture
    // orders (the donate flow) it precedes ORDER_COMPLETED by ms and
    // there's no useful action between them.
    match event.event.as_str() {
        "ORDER_COMPLETED" => {
            if let Err(e) = state
                .orders
                .mark_state(&event.order_id, OrderState::Completed)
                .await
            {
                tracing::error!(error = %e, "mark_state(completed) failed");
                return err(StatusCode::INTERNAL_SERVER_ERROR, "webhook_state_failed");
            }
            let coverage_until = Utc::now() + chrono::Duration::days(COVERAGE_DAYS);
            if let Err(e) = state
                .supporters
                .mark_payment_received(
                    row.user_id,
                    row.name_plate_at_checkout.as_deref(),
                    coverage_until,
                )
                .await
            {
                tracing::error!(error = %e, "mark_payment_received failed");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "webhook_supporter_failed",
                );
            }
            (
                StatusCode::OK,
                Json(WebhookAck {
                    status: "completed".into(),
                }),
            )
                .into_response()
        }
        "ORDER_FAILED" => {
            if let Err(e) = state
                .orders
                .mark_state(&event.order_id, OrderState::Failed)
                .await
            {
                tracing::error!(error = %e, "mark_state(failed) failed");
                return err(StatusCode::INTERNAL_SERVER_ERROR, "webhook_state_failed");
            }
            (
                StatusCode::OK,
                Json(WebhookAck {
                    status: "failed".into(),
                }),
            )
                .into_response()
        }
        "ORDER_CANCELLED" => {
            if let Err(e) = state
                .orders
                .mark_state(&event.order_id, OrderState::Cancelled)
                .await
            {
                tracing::error!(error = %e, "mark_state(cancelled) failed");
                return err(StatusCode::INTERNAL_SERVER_ERROR, "webhook_state_failed");
            }
            (
                StatusCode::OK,
                Json(WebhookAck {
                    status: "cancelled".into(),
                }),
            )
                .into_response()
        }
        _ => (
            StatusCode::OK,
            Json(WebhookAck {
                status: "ignored".into(),
            }),
        )
            .into_response(),
    }
}

/// Construct the route state from the optional [`RevolutConfig`].
/// Even when `revolut` is `None` we still mount the routes (with the
/// 503 short-circuit) so the OpenAPI surface is stable across
/// configurations.
pub fn build_state(
    orders: Arc<PostgresOrderStore>,
    supporters: Arc<PostgresSupporterStore>,
    cfg: Option<&RevolutConfig>,
) -> DonateState {
    let revolut = cfg.map(|c| {
        Arc::new(RevolutClient::new(
            c.api_base.clone(),
            c.api_key.clone(),
            c.api_version.clone(),
        ))
    });
    let webhook_secret = cfg.map(|c| c.webhook_secret.as_bytes().to_vec());
    let return_url = cfg.and_then(|c| c.return_url.clone());
    DonateState {
        orders,
        supporters,
        revolut,
        webhook_secret,
        return_url,
    }
}
