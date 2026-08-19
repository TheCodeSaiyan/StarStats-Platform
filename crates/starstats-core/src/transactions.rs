//! Transaction pairing — derive higher-order facts from the
//! request/response event pairs the parser surfaces.
//!
//! The Star Citizen client emits commerce as two log lines:
//!
//! 1. `Send{Shop,Commodity}*Request` — the player clicked Buy/Sell.
//!    Optimistic; the server may still reject.
//! 2. `ShopFlowResponse` — server confirmation (or rejection).
//!    Currently only observed for shop events; commodity terminals
//!    have no confirmed response shape in our regex set.
//!
//! This module joins them into a [`Transaction`] aggregate so the UI
//! can show "Bought helmet — confirmed 0.4s later" instead of two
//! disconnected timeline rows.
//!
//! Pure, allocation-light, no I/O. Callers feed it a slice of recent
//! events plus a `now` timestamp (the wall-clock anchor used to age
//! out unmatched requests into [`TransactionStatus::TimedOut`]).

use crate::events::{
    CommodityBuyRequest, CommoditySellRequest, GameEvent, ShopBuyRequest, ShopFlowResponse,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    Shop,
    CommodityBuy,
    CommoditySell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    /// Request seen, no response yet, request still inside the window.
    Pending,
    /// Server response with a success token.
    Confirmed,
    /// Server response with a non-success token.
    Rejected,
    /// Request seen, no response observed, older than the window.
    TimedOut,
    /// Commodity (or any kind) where we never expect a response in
    /// the parser surface area — surfaced as "submitted" rather than
    /// pending forever.
    Submitted,
}

/// Joined request + response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub kind: TransactionKind,
    pub status: TransactionStatus,
    pub started_at: String,
    pub confirmed_at: Option<String>,
    pub shop_id: Option<String>,
    pub item: Option<String>,
    pub quantity: Option<f64>,
    /// Raw body of the request, kept verbatim for forensics.
    pub raw_request: String,
    pub raw_response: Option<String>,
}

/// The `event_type` discriminants (snake_case, matching `GameEvent`'s
/// `#[serde(tag = "type", rename_all = "snake_case")]`) that
/// [`pair_transactions`] consumes. Colocated here so a server-side query
/// can fetch exactly the commerce events (all of them — they're rare
/// per user) instead of scanning the whole event stream. MUST stay in
/// sync with the match arms in [`pair_transactions`]: add a variant
/// there, add its type string here.
pub const COMMERCE_EVENT_TYPES: &[&str] = &[
    "shop_buy_request",
    "shop_flow_response",
    "commodity_buy_request",
    "commodity_sell_request",
];

/// Pair shop request/response events and surface commodity requests
/// as standalone "submitted" rows.
///
/// `events` does not need to be sorted — we sort by `started_at`
/// internally for the response-pairing scan. `now` is an ISO-ish
/// timestamp string; if it's empty or unparseable, requests without
/// responses stay [`TransactionStatus::Pending`] indefinitely (no
/// time math is performed).
///
/// `window_secs` is the "if we haven't heard back in N seconds, mark
/// it timed out" threshold. ~30s is a reasonable default; anything
/// the server hasn't responded to in 30s is realistically lost.
pub fn pair_transactions(events: &[GameEvent], now: &str, window_secs: i64) -> Vec<Transaction> {
    let mut shop_requests: Vec<(usize, &ShopBuyRequest)> = Vec::new();
    let mut shop_responses: Vec<&ShopFlowResponse> = Vec::new();
    let mut commodity_buys: Vec<&CommodityBuyRequest> = Vec::new();
    let mut commodity_sells: Vec<&CommoditySellRequest> = Vec::new();

    for ev in events {
        match ev {
            GameEvent::ShopBuyRequest(r) => shop_requests.push((shop_requests.len(), r)),
            GameEvent::ShopFlowResponse(r) => shop_responses.push(r),
            GameEvent::CommodityBuyRequest(r) => commodity_buys.push(r),
            GameEvent::CommoditySellRequest(r) => commodity_sells.push(r),
            _ => {}
        }
    }

    // Match each response to the most-recent earlier request with the
    // same shop_id (or any pending if shop_id absent on either side).
    // Once paired, both sides get consumed via a parallel `paired_resp`
    // array — we don't mutate the request list because we need it
    // again for emission below.
    let mut paired_resp: Vec<Option<&ShopFlowResponse>> = vec![None; shop_requests.len()];
    for resp in &shop_responses {
        let resp_shop = resp.shop_id.as_deref();
        let pick = shop_requests
            .iter()
            .enumerate()
            .filter(|(idx, (_, req))| {
                if paired_resp[*idx].is_some() {
                    return false;
                }
                match (resp_shop, req.shop_id.as_deref()) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                }
            })
            .filter(|(_, (_, req))| req.timestamp <= resp.timestamp)
            .max_by(|a, b| a.1 .1.timestamp.cmp(&b.1 .1.timestamp));

        if let Some((idx, _)) = pick {
            paired_resp[idx] = Some(resp);
        }
    }

    let now_secs = parse_iso_secs(now);

    let mut out: Vec<Transaction> = Vec::with_capacity(events.len());

    for (idx, (_, req)) in shop_requests.iter().enumerate() {
        let resp = paired_resp[idx];
        let status = match (resp, resp.and_then(|r| r.success)) {
            (Some(_), Some(true)) => TransactionStatus::Confirmed,
            (Some(_), Some(false)) => TransactionStatus::Rejected,
            (Some(_), None) => TransactionStatus::Confirmed, // ack with no result token = treat as confirmed
            (None, _) => age_status(&req.timestamp, now_secs, window_secs),
        };
        out.push(Transaction {
            kind: TransactionKind::Shop,
            status,
            started_at: req.timestamp.clone(),
            confirmed_at: resp.map(|r| r.timestamp.clone()),
            shop_id: req.shop_id.clone(),
            item: req.item_class.clone(),
            quantity: req.quantity.map(|q| q as f64),
            raw_request: req.raw.clone(),
            raw_response: resp.map(|r| r.raw.clone()),
        });
    }

    for buy in &commodity_buys {
        out.push(Transaction {
            kind: TransactionKind::CommodityBuy,
            status: TransactionStatus::Submitted,
            started_at: buy.timestamp.clone(),
            confirmed_at: None,
            shop_id: None,
            item: buy.commodity.clone(),
            quantity: buy.quantity,
            raw_request: buy.raw.clone(),
            raw_response: None,
        });
    }

    for sell in &commodity_sells {
        out.push(Transaction {
            kind: TransactionKind::CommoditySell,
            status: TransactionStatus::Submitted,
            started_at: sell.timestamp.clone(),
            confirmed_at: None,
            shop_id: None,
            item: sell.commodity.clone(),
            quantity: sell.quantity,
            raw_request: sell.raw.clone(),
            raw_response: None,
        });
    }

    out.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    out
}

/// If we can extract a unix-second from `started_at` and `now_secs`
/// is `Some`, age the request: older than the window → TimedOut.
/// Otherwise leave it as Pending — a missing clock is still useful
/// telemetry, just not actionable for ageing.
fn age_status(started_at: &str, now_secs: Option<i64>, window_secs: i64) -> TransactionStatus {
    match (now_secs, parse_iso_secs(started_at)) {
        (Some(now), Some(start)) if now - start > window_secs => TransactionStatus::TimedOut,
        _ => TransactionStatus::Pending,
    }
}

/// Permissive ISO-8601-ish → unix-seconds extractor. Accepts:
///   `2026-05-07T14:00:00.000Z`
///   `2026-05-07T14:00:00+00:00`
///   `2026-05-07 14:00:00.789` (launcher format)
///
/// Returns `None` when the string doesn't look parseable. We do this
/// by hand instead of pulling `chrono` because `starstats-core`
/// keeps its dep set lean.
fn parse_iso_secs(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    // Replace separator + drop fractional and offset for a cheap
    // year/month/day/hour/min/sec read.
    let normalised = s.replace('T', " ");
    let primary = normalised.split('.').next().unwrap_or(&normalised);
    let primary = primary
        .split('+')
        .next()
        .unwrap_or(primary)
        .split('Z')
        .next()
        .unwrap_or(primary)
        .trim();
    let mut parts = primary.split([' ', '-', ':']);
    let y: i64 = parts.next()?.parse().ok()?;
    let mo: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let h: i64 = parts.next()?.parse().ok()?;
    let mi: i64 = parts.next()?.parse().ok()?;
    let se: i64 = parts.next()?.parse().ok()?;
    // Days-from-civil — no DST or leap-second nuance, fine for "is
    // this 30s old".
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

/// Howard Hinnant's days_from_civil algorithm, narrowed to i64.
/// Returns days since 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m_u = m as u64;
    let d_u = d as u64;
    let doy = (153 * if m_u > 2 { m_u - 3 } else { m_u + 9 } + 2) / 5 + d_u - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ShopBuyRequest;

    fn shop_buy(ts: &str, shop: Option<&str>, item: Option<&str>) -> GameEvent {
        GameEvent::ShopBuyRequest(ShopBuyRequest {
            timestamp: ts.to_string(),
            shop_id: shop.map(String::from),
            item_class: item.map(String::from),
            quantity: Some(1),
            raw: "SendShopBuyRequest(...)".to_string(),
            price: None,
            shop_name: None,
        })
    }

    fn shop_resp(ts: &str, shop: Option<&str>, success: Option<bool>) -> GameEvent {
        GameEvent::ShopFlowResponse(ShopFlowResponse {
            timestamp: ts.to_string(),
            shop_id: shop.map(String::from),
            success,
            raw: "ShopFlowResponse(...)".to_string(),
        })
    }

    #[test]
    fn pairs_shop_request_with_response_by_shop_id() {
        let events = vec![
            shop_buy("2026-05-07T13:00:00.000Z", Some("kiosk_a"), Some("helmet")),
            shop_resp("2026-05-07T13:00:01.000Z", Some("kiosk_a"), Some(true)),
        ];
        let txs = pair_transactions(&events, "2026-05-07T13:01:00.000Z", 30);
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].status, TransactionStatus::Confirmed);
        assert_eq!(
            txs[0].confirmed_at.as_deref(),
            Some("2026-05-07T13:00:01.000Z")
        );
    }

    #[test]
    fn marks_unanswered_request_timed_out_after_window() {
        let events = vec![shop_buy(
            "2026-05-07T13:00:00.000Z",
            Some("kiosk_a"),
            Some("helmet"),
        )];
        // 'now' is 60s after the request → > 30s window.
        let txs = pair_transactions(&events, "2026-05-07T13:01:00.000Z", 30);
        assert_eq!(txs[0].status, TransactionStatus::TimedOut);
    }

    #[test]
    fn keeps_recent_unanswered_request_pending() {
        let events = vec![shop_buy(
            "2026-05-07T13:00:00.000Z",
            Some("kiosk_a"),
            Some("helmet"),
        )];
        // 'now' is 5s after the request → still inside 30s window.
        let txs = pair_transactions(&events, "2026-05-07T13:00:05.000Z", 30);
        assert_eq!(txs[0].status, TransactionStatus::Pending);
    }

    #[test]
    fn rejected_response_marks_transaction_rejected() {
        let events = vec![
            shop_buy("2026-05-07T13:00:00.000Z", Some("kiosk_a"), Some("helmet")),
            shop_resp("2026-05-07T13:00:01.000Z", Some("kiosk_a"), Some(false)),
        ];
        let txs = pair_transactions(&events, "2026-05-07T13:01:00.000Z", 30);
        assert_eq!(txs[0].status, TransactionStatus::Rejected);
    }

    #[test]
    fn surfaces_commodity_buy_as_submitted() {
        let events = vec![GameEvent::CommodityBuyRequest(CommodityBuyRequest {
            timestamp: "2026-05-07T14:00:00.000Z".to_string(),
            commodity: Some("Agricium".to_string()),
            quantity: Some(125.5),
            raw: "SendCommodityBuyRequest(...)".to_string(),
        })];
        let txs = pair_transactions(&events, "2026-05-07T14:01:00.000Z", 30);
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].kind, TransactionKind::CommodityBuy);
        assert_eq!(txs[0].status, TransactionStatus::Submitted);
        assert_eq!(txs[0].quantity, Some(125.5));
    }

    #[test]
    fn pairs_in_arrival_order_when_multiple_pending() {
        // Two requests to the same shop, two responses — earlier req
        // pairs with earlier resp.
        let events = vec![
            shop_buy("2026-05-07T13:00:00.000Z", Some("kiosk_a"), Some("helmet")),
            shop_buy("2026-05-07T13:00:02.000Z", Some("kiosk_a"), Some("boots")),
            shop_resp("2026-05-07T13:00:03.000Z", Some("kiosk_a"), Some(true)),
            shop_resp("2026-05-07T13:00:04.000Z", Some("kiosk_a"), Some(true)),
        ];
        let txs = pair_transactions(&events, "2026-05-07T13:01:00.000Z", 30);
        assert_eq!(txs.len(), 2);
        for tx in &txs {
            assert_eq!(tx.status, TransactionStatus::Confirmed);
        }
    }
}
