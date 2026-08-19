//! Revolut Business Merchant API client + webhook signature
//! verification.
//!
//! See `docs/REVOLUT-INTEGRATION-PLAN.md` for the wider integration
//! plan. This module is the wire-level shim that:
//!
//!   1. POSTs to `{base}/api/1.0/orders` to create a hosted-checkout
//!      order, returning the URL we redirect the customer to.
//!   2. Verifies HMAC-SHA256 signatures on incoming webhook events
//!      so we can trust the `ORDER_COMPLETED` notifications that
//!      flip a user's supporter state.
//!
//! The Revolut webhook signature scheme (per Revolut's docs):
//!
//!   payload_to_sign = "v1." + Revolut-Request-Timestamp + "." + raw_body
//!   signature       = "v1=" + hex(hmac_sha256(secret, payload_to_sign))
//!
//! The `Revolut-Signature` header may carry multiple comma-separated
//! `v1=...` signatures (during signing-secret rotations); we accept
//! any of them, with the timestamp drift capped at ±5 minutes.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Maximum tolerance between the timestamp Revolut signed and our
/// current clock. 5 minutes is what Revolut's own example rejects
/// against, so we mirror that.
pub const TIMESTAMP_DRIFT_TOLERANCE: chrono::Duration = chrono::Duration::minutes(5);

#[derive(Debug, thiserror::Error)]
pub enum RevolutError {
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Revolut returned non-2xx ({status}): {body}")]
    Status { status: u16, body: String },
    #[error("Revolut returned a malformed response: {0}")]
    Malformed(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("Revolut-Signature header missing or empty")]
    HeaderMissing,
    #[error("Revolut-Request-Timestamp header missing or invalid")]
    TimestampMissing,
    #[error("Revolut-Signature did not contain any v1= entry we could parse")]
    NoSupportedScheme,
    #[error("timestamp drift exceeds the 5-minute tolerance")]
    TimestampOutOfRange,
    #[error("signature mismatch (none of the supplied v1= entries verified)")]
    Mismatch,
}

/// Wire shape of `POST /api/1.0/orders`. We only carry the fields we
/// actually use; Revolut accepts plenty more (line items, customer
/// objects, etc.) but for a donate flow they're unnecessary.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOrderRequest<'a> {
    /// Amount in minor units (pence, cents, etc.) per ISO 4217.
    pub amount: i64,
    /// Three-letter ISO 4217 code, uppercase.
    pub currency: &'a str,
    /// Where Revolut redirects after a successful payment. Optional —
    /// when absent Revolut uses the merchant default configured in
    /// the Business dashboard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<&'a str>,
    /// Free-form string Revolut echoes back on every webhook for the
    /// order — we set it to our internal order UUID so the webhook
    /// handler can correlate without a pre-fetched lookup table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merchant_order_ext_ref: Option<&'a str>,
    /// Visible on the customer's bank statement (where supported)
    /// and on the Revolut hosted checkout page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'a str>,
}

/// Wire shape of the create-order response. We deserialise only the
/// fields we need; Revolut may include more.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateOrderResponse {
    /// Revolut's own order id — different from our
    /// `merchant_order_ext_ref`. Stash this for webhook correlation.
    pub id: String,
    /// Customer-visible URL to send the user to.
    pub checkout_url: String,
    /// `pending` for a freshly-created order. We don't act on this
    /// directly; the webhook is the source of truth for completion.
    #[serde(default)]
    #[allow(dead_code)]
    pub state: String,
}

/// Wire shape of inbound webhook events. Revolut sends `event` as the
/// type slug and `order_id` (their id, not ours) plus our echo via
/// `merchant_order_ext_ref`.
#[derive(Debug, Clone, Deserialize)]
pub struct WebhookEvent {
    pub event: String,
    pub order_id: String,
    /// Echoes our internal id from `CreateOrderRequest::merchant_order_ext_ref`.
    /// Optional because not every event carries one. Currently unread —
    /// the route layer looks up by `order_id` instead — but kept on the
    /// wire shape for forward-compat (future events may not carry an
    /// `order_id` we already know about, in which case we'd fall back
    /// to this).
    #[serde(default)]
    #[allow(dead_code)]
    pub merchant_order_ext_ref: Option<String>,
}

#[derive(Clone)]
pub struct RevolutClient {
    http: Client,
    api_base: String,
    api_key: String,
    api_version: String,
}

impl RevolutClient {
    pub fn new(api_base: String, api_key: String, api_version: String) -> Self {
        // 30s aligns with the rest of our outbound clients (RSI scrape,
        // SpiceDB). Revolut's own SLA is sub-second for order create;
        // anything slower is a problem worth surfacing as an error
        // rather than holding the connection.
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("build reqwest client");
        Self {
            http,
            api_base,
            api_key,
            api_version,
        }
    }

    pub async fn create_order(
        &self,
        req: &CreateOrderRequest<'_>,
    ) -> Result<CreateOrderResponse, RevolutError> {
        let url = format!("{}/api/1.0/orders", self.api_base);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("Revolut-Api-Version", &self.api_version)
            .json(req)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(RevolutError::Status {
                status: status.as_u16(),
                body,
            });
        }
        let parsed: CreateOrderResponse = serde_json::from_str(&body)?;
        Ok(parsed)
    }
}

/// Verify a Revolut webhook payload signature in constant time.
///
/// `raw_body` MUST be the byte-exact request body Revolut delivered —
/// no whitespace re-formatting, no JSON re-serialisation. The signing
/// payload is `v1.<timestamp>.<raw_body>` and any divergence in the
/// body (even a re-serialised key order) breaks the HMAC.
///
/// The `signature_header` may contain a single `v1=<hex>` entry or a
/// comma-separated list of them (Revolut emits multiple during
/// secret rotations). We accept the event if any one entry verifies.
pub fn verify_webhook_signature(
    secret: &[u8],
    timestamp_header: &str,
    signature_header: &str,
    raw_body: &[u8],
    now: DateTime<Utc>,
) -> Result<(), SignatureError> {
    if signature_header.trim().is_empty() {
        return Err(SignatureError::HeaderMissing);
    }
    let timestamp_ms: i64 = timestamp_header
        .trim()
        .parse()
        .map_err(|_| SignatureError::TimestampMissing)?;
    let event_ts = DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or(SignatureError::TimestampMissing)?;
    let drift = (now - event_ts).abs();
    if drift > TIMESTAMP_DRIFT_TOLERANCE {
        return Err(SignatureError::TimestampOutOfRange);
    }

    // Construct the expected HMAC bytes once. We compare against each
    // candidate `v1=...` entry; any match means the event is genuine.
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(b"v1.");
    mac.update(timestamp_header.trim().as_bytes());
    mac.update(b".");
    mac.update(raw_body);
    let expected = mac.finalize().into_bytes();

    let mut saw_v1 = false;
    for entry in signature_header.split(',') {
        let trimmed = entry.trim();
        let Some(hex_part) = trimmed.strip_prefix("v1=") else {
            // Future schemes (v2= etc.) are tolerated but ignored.
            continue;
        };
        saw_v1 = true;
        let Ok(decoded) = hex::decode(hex_part) else {
            continue;
        };
        if decoded.len() != expected.len() {
            continue;
        }
        // Constant-time comparison so a timing attack can't leak the
        // expected MAC byte-by-byte. `subtle::ConstantTimeEq` returns
        // a `Choice` whose `unwrap_u8() == 1` iff equal.
        if decoded.ct_eq(expected.as_slice()).into() {
            return Ok(());
        }
    }
    if !saw_v1 {
        return Err(SignatureError::NoSupportedScheme);
    }
    Err(SignatureError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the canonical `v1=<hex>` signature for a body+timestamp
    /// pair using the same algorithm the live verifier expects.
    fn sign(secret: &[u8], timestamp: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
        mac.update(b"v1.");
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(body);
        format!("v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn now_ms_str(now: DateTime<Utc>) -> String {
        now.timestamp_millis().to_string()
    }

    #[test]
    fn verify_accepts_valid_signature() {
        let secret = b"wsk_testsecret";
        let now = Utc::now();
        let ts = now_ms_str(now);
        let body =
            br#"{"event":"ORDER_COMPLETED","order_id":"abc","merchant_order_ext_ref":"xyz"}"#;
        let sig = sign(secret, &ts, body);
        verify_webhook_signature(secret, &ts, &sig, body, now).expect("verify ok");
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let secret = b"wsk_testsecret";
        let now = Utc::now();
        let ts = now_ms_str(now);
        let body = br#"{"event":"ORDER_COMPLETED"}"#;
        let sig = sign(secret, &ts, body);
        let tampered = br#"{"event":"ORDER_FAILED"}"#;
        let err = verify_webhook_signature(secret, &ts, &sig, tampered, now).unwrap_err();
        assert_eq!(err, SignatureError::Mismatch);
    }

    #[test]
    fn verify_rejects_wrong_secret() {
        let now = Utc::now();
        let ts = now_ms_str(now);
        let body = br#"{}"#;
        let sig = sign(b"correct", &ts, body);
        let err = verify_webhook_signature(b"wrong", &ts, &sig, body, now).unwrap_err();
        assert_eq!(err, SignatureError::Mismatch);
    }

    #[test]
    fn verify_rejects_stale_timestamp() {
        let secret = b"wsk_testsecret";
        let now = Utc::now();
        let stale = now - chrono::Duration::minutes(10);
        let ts = now_ms_str(stale);
        let body = br#"{}"#;
        let sig = sign(secret, &ts, body);
        let err = verify_webhook_signature(secret, &ts, &sig, body, now).unwrap_err();
        assert_eq!(err, SignatureError::TimestampOutOfRange);
    }

    #[test]
    fn verify_rejects_future_timestamp_beyond_tolerance() {
        let secret = b"wsk_testsecret";
        let now = Utc::now();
        let future = now + chrono::Duration::minutes(10);
        let ts = now_ms_str(future);
        let body = br#"{}"#;
        let sig = sign(secret, &ts, body);
        let err = verify_webhook_signature(secret, &ts, &sig, body, now).unwrap_err();
        assert_eq!(err, SignatureError::TimestampOutOfRange);
    }

    #[test]
    fn verify_accepts_any_of_multiple_signatures() {
        let secret = b"wsk_testsecret";
        let now = Utc::now();
        let ts = now_ms_str(now);
        let body = br#"{}"#;
        let good = sign(secret, &ts, body);
        let bad = "v1=deadbeef".to_string();
        // Order: bad first, good second. Verifier should still accept.
        let header = format!("{bad},{good}");
        verify_webhook_signature(secret, &ts, &header, body, now).expect("verify ok");
    }

    #[test]
    fn verify_rejects_when_no_v1_entry() {
        let secret = b"wsk_testsecret";
        let now = Utc::now();
        let ts = now_ms_str(now);
        let body = br#"{}"#;
        let err = verify_webhook_signature(secret, &ts, "v2=abcdef", body, now).unwrap_err();
        assert_eq!(err, SignatureError::NoSupportedScheme);
    }

    #[test]
    fn verify_rejects_empty_signature_header() {
        let secret = b"wsk_testsecret";
        let now = Utc::now();
        let ts = now_ms_str(now);
        let body = br#"{}"#;
        let err = verify_webhook_signature(secret, &ts, "", body, now).unwrap_err();
        assert_eq!(err, SignatureError::HeaderMissing);
    }

    #[test]
    fn verify_rejects_non_numeric_timestamp() {
        let secret = b"wsk_testsecret";
        let now = Utc::now();
        let body = br#"{}"#;
        let sig = sign(secret, "1683650202360", body);
        let err = verify_webhook_signature(secret, "not-a-number", &sig, body, now).unwrap_err();
        assert_eq!(err, SignatureError::TimestampMissing);
    }
}
