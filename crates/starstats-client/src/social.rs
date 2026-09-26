//! Tray-side friends, blocks, mutes and notifications.
//!
//! Every call runs Rust-side, for the same reason as `whats_new.rs`: the
//! WebView CSP blocks cross-origin `fetch()`. The DTOs mirror the server's
//! `social_routes` / `social` / `notifications` shapes so the renderer can
//! deserialise what a command returns without a shared schema crate.
//!
//! The tray authenticates with its DEVICE token. The server resolves the
//! caller from the token's `sub` (the owning user), not from
//! `preferred_username` (the device label), so these calls act as the
//! player, not as "Hangar PC".
//!
//! Also here: the notification poller, which turns new unread notifications
//! into OS toasts, and the cached friends list that `gamelog` hands to
//! `detect_pii` so a friend's handle is redacted in a submitted log line.

use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Poll cadence. A friend request is not urgent, and every tray polling
/// every minute is the whole load this feature puts on the server until the
/// realtime gateway replaces it.
pub const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Above this many new notifications in one tick, show one summary toast
/// rather than a stack of them.
const TOAST_SUMMARY_THRESHOLD: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum SocialClientError {
    #[error("tray is not paired (no api_url or token)")]
    NotPaired,
    /// A non-2xx from the server, with its `error` code when it sent one.
    /// The code is what the pane maps to copy (`user_not_found`, ...).
    #[error("{code}")]
    Api {
        status: reqwest::StatusCode,
        code: String,
    },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// -- Wire DTOs (mirror the server) ------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Friend {
    pub handle: String,
    pub since: DateTime<Utc>,
    pub rsi_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FriendRequest {
    pub id: Uuid,
    pub requester_handle: String,
    pub recipient_handle: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FriendsResponse {
    pub friends: Vec<Friend>,
    pub incoming: Vec<FriendRequest>,
    pub outgoing: Vec<FriendRequest>,
    pub friend_request_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SendFriendRequestResponse {
    /// `requested` or `became_friends`.
    pub outcome: String,
    pub request: Option<FriendRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListedHandle {
    pub handle: String,
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlocksResponse {
    pub blocks: Vec<ListedHandle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MutesResponse {
    pub mutes: Vec<ListedHandle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SocialSettings {
    pub friend_request_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Notification {
    pub id: Uuid,
    /// `friend_request` or `friend_accepted`.
    pub kind: String,
    pub actor_handle: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotificationsResponse {
    pub items: Vec<Notification>,
    pub unread_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkReadResponse {
    pub updated: u64,
    pub unread_count: i64,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

// -- Client -------------------------------------------------------------

/// Built per call from the current config, so pairing and unpairing take
/// effect on the next request with no cache to invalidate.
pub struct SocialClient {
    http: Client,
    api_url: String,
    bearer: String,
}

/// Percent-encode a handle for a path segment. Handles are validated to
/// `[A-Za-z0-9_-]` server-side, so this only matters for input the server
/// will reject anyway, but it must never be able to change the path.
fn seg(s: &str) -> String {
    s.trim()
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

impl SocialClient {
    pub fn from_config(cfg: &crate::config::Config) -> Result<Self, SocialClientError> {
        let api_url = cfg
            .remote_sync
            .api_url
            .as_deref()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .ok_or(SocialClientError::NotPaired)?;
        // Unlike What's New there is no anonymous mode: every call is
        // about the signed-in player.
        let bearer = cfg
            .remote_sync
            .access_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or(SocialClientError::NotPaired)?;
        let http = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("build http client")?;
        Ok(Self {
            http,
            api_url,
            bearer,
        })
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response, SocialClientError> {
        let url = format!("{}{}", self.api_url, path);
        let mut req = self
            .http
            .request(method.clone(), &url)
            .bearer_auth(&self.bearer);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("send {method} {path}"))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let code = resp
            .json::<ErrorBody>()
            .await
            .map(|b| b.error)
            .unwrap_or_else(|_| format!("http_{}", status.as_u16()));
        Err(SocialClientError::Api { status, code })
    }

    async fn json<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, SocialClientError> {
        let resp = self.send(method, path, body).await?;
        Ok(resp
            .json::<T>()
            .await
            .with_context(|| format!("decode {path}"))?)
    }

    async fn empty(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(), SocialClientError> {
        self.send(method, path, body).await.map(|_| ())
    }

    pub async fn friends(&self) -> Result<FriendsResponse, SocialClientError> {
        self.json(Method::GET, "/v1/me/friends", None).await
    }

    pub async fn send_request(
        &self,
        handle: &str,
    ) -> Result<SendFriendRequestResponse, SocialClientError> {
        let handle = handle.trim().trim_start_matches('@');
        self.json(
            Method::POST,
            "/v1/me/friends/requests",
            Some(serde_json::json!({ "handle": handle })),
        )
        .await
    }

    pub async fn respond(&self, id: Uuid, action: &str) -> Result<(), SocialClientError> {
        if !matches!(action, "accept" | "decline" | "cancel") {
            return Err(anyhow::anyhow!("unknown request action {action:?}").into());
        }
        self.empty(
            Method::POST,
            &format!("/v1/me/friends/requests/{id}/{action}"),
            None,
        )
        .await
    }

    pub async fn remove_friend(&self, handle: &str) -> Result<(), SocialClientError> {
        self.empty(
            Method::DELETE,
            &format!("/v1/me/friends/{}", seg(handle)),
            None,
        )
        .await
    }

    pub async fn blocks(&self) -> Result<BlocksResponse, SocialClientError> {
        self.json(Method::GET, "/v1/me/blocks", None).await
    }

    pub async fn set_blocked(&self, handle: &str, blocked: bool) -> Result<(), SocialClientError> {
        let method = if blocked { Method::PUT } else { Method::DELETE };
        self.empty(method, &format!("/v1/me/blocks/{}", seg(handle)), None)
            .await
    }

    pub async fn mutes(&self) -> Result<MutesResponse, SocialClientError> {
        self.json(Method::GET, "/v1/me/mutes", None).await
    }

    pub async fn set_muted(&self, handle: &str, muted: bool) -> Result<(), SocialClientError> {
        let method = if muted { Method::PUT } else { Method::DELETE };
        self.empty(method, &format!("/v1/me/mutes/{}", seg(handle)), None)
            .await
    }

    pub async fn update_settings(
        &self,
        friend_request_policy: &str,
    ) -> Result<SocialSettings, SocialClientError> {
        self.json(
            Method::PUT,
            "/v1/me/social/settings",
            Some(serde_json::json!({ "friend_request_policy": friend_request_policy })),
        )
        .await
    }

    pub async fn notifications(
        &self,
        since: Option<DateTime<Utc>>,
        limit: u32,
    ) -> Result<NotificationsResponse, SocialClientError> {
        let mut path = format!("/v1/me/notifications?limit={limit}");
        if let Some(s) = since {
            // `Z` form: no `+` to escape in the query string.
            path.push_str(&format!(
                "&since={}",
                s.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            ));
        }
        self.json(Method::GET, &path, None).await
    }

    pub async fn mark_read(
        &self,
        ids: &[Uuid],
        all: bool,
    ) -> Result<MarkReadResponse, SocialClientError> {
        let body = if all {
            serde_json::json!({ "all": true })
        } else {
            serde_json::json!({ "ids": ids })
        };
        self.json(Method::POST, "/v1/me/notifications/read", Some(body))
            .await
    }
}

// -- Known friends, for PII redaction ------------------------------------

fn known_friends_cell() -> &'static RwLock<Vec<String>> {
    static CELL: OnceLock<RwLock<Vec<String>>> = OnceLock::new();
    CELL.get_or_init(|| RwLock::new(Vec::new()))
}

/// The friends list as of the last successful poll. Empty until then, and
/// empty when unpaired; redaction degrades to "own handle only" exactly as
/// it did before friends existed.
pub fn known_friends() -> Vec<String> {
    known_friends_cell()
        .read()
        .map(|g| g.clone())
        .unwrap_or_default()
}

fn set_known_friends(handles: Vec<String>) {
    if let Ok(mut g) = known_friends_cell().write() {
        *g = handles;
    }
}

// -- Notification poller ---------------------------------------------------

/// What to do with the notifications that arrived since the watermark.
#[derive(Debug, PartialEq)]
pub enum ToastPlan {
    Nothing,
    /// Hold them: the player is in game and asked not to be interrupted.
    /// The watermark does not move, so they toast once the game closes.
    Hold,
    Each(Vec<(String, String)>),
    Summary(usize),
}

/// Title and body for one notification, or `None` for a kind this build
/// does not know (a newer server); unknown kinds are counted, not shown.
pub fn toast_text(n: &Notification) -> Option<(String, String)> {
    let who = n.actor_handle.as_deref().unwrap_or("Someone");
    match n.kind.as_str() {
        "friend_request" => Some((
            "Friend request".to_string(),
            format!("@{who} wants to be friends. Open StarStats to accept."),
        )),
        "friend_accepted" => Some((
            "Friend request accepted".to_string(),
            format!("@{who} accepted your friend request."),
        )),
        _ => None,
    }
}

/// Decide the toasts for one poll. Pure, so the rules are testable without
/// a runtime: only unread rows newer than the watermark count, and a burst
/// collapses into one summary.
pub fn plan_toasts(
    items: &[Notification],
    watermark: DateTime<Utc>,
    in_game_quiet: bool,
) -> ToastPlan {
    let fresh: Vec<&Notification> = items
        .iter()
        .filter(|n| n.read_at.is_none() && n.created_at > watermark)
        .collect();
    if fresh.is_empty() {
        return ToastPlan::Nothing;
    }
    if in_game_quiet {
        return ToastPlan::Hold;
    }
    if fresh.len() > TOAST_SUMMARY_THRESHOLD {
        return ToastPlan::Summary(fresh.len());
    }
    let each: Vec<(String, String)> = fresh.iter().filter_map(|n| toast_text(n)).collect();
    if each.is_empty() {
        ToastPlan::Nothing
    } else {
        ToastPlan::Each(each)
    }
}

/// Payload of the `social-notifications` event the pane listens for.
#[derive(Debug, Clone, Serialize)]
pub struct SocialNotificationsEvent {
    pub unread_count: i64,
}

/// Poll forever. Config is reloaded every tick so pairing, unpairing and the
/// toast settings apply without a restart. The first successful poll only
/// sets the watermark: notifications that were already waiting at launch are
/// shown in the pane and badge, not replayed as a burst of toasts.
pub async fn run_poller(app: tauri::AppHandle) {
    use tauri::Emitter;
    use tauri_plugin_notification::NotificationExt;

    let mut watermark: Option<DateTime<Utc>> = None;
    let mut ticks: u64 = 0;
    loop {
        ticks += 1;
        let cfg = match crate::config::load() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(error = %e, "social poll: config load failed");
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
        };
        let client = match SocialClient::from_config(&cfg) {
            Ok(c) => c,
            Err(_) => {
                // Unpaired: nothing to poll, and nothing to redact with.
                set_known_friends(Vec::new());
                watermark = None;
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
        };

        // The friends list changes rarely; refresh it every tenth tick.
        if ticks % 10 == 1 {
            match client.friends().await {
                Ok(f) => set_known_friends(f.friends.into_iter().map(|f| f.handle).collect()),
                Err(e) => tracing::debug!(error = %e, "social poll: friends fetch failed"),
            }
        }

        match client.notifications(None, 20).await {
            Ok(resp) => {
                let _ = app.emit(
                    "social-notifications",
                    SocialNotificationsEvent {
                        unread_count: resp.unread_count,
                    },
                );
                let newest = resp.items.iter().map(|n| n.created_at).max();
                match watermark {
                    None => watermark = Some(newest.unwrap_or_else(Utc::now)),
                    Some(w) => {
                        let quiet = cfg.social.quiet_in_game
                            && crate::process_guard::is_starcitizen_running();
                        let plan = if cfg.social.toasts {
                            plan_toasts(&resp.items, w, quiet)
                        } else {
                            ToastPlan::Nothing
                        };
                        let show = |title: &str, body: &str| {
                            if let Err(e) =
                                app.notification().builder().title(title).body(body).show()
                            {
                                tracing::warn!(error = %e, "social toast failed");
                            }
                        };
                        match &plan {
                            ToastPlan::Each(list) => {
                                for (t, b) in list {
                                    show(t, b);
                                }
                            }
                            ToastPlan::Summary(n) => show(
                                "StarStats",
                                &format!(
                                    "{n} new friend notifications. Open StarStats to see them."
                                ),
                            ),
                            ToastPlan::Nothing | ToastPlan::Hold => {}
                        }
                        if plan != ToastPlan::Hold {
                            if let Some(n) = newest {
                                watermark = Some(n.max(w));
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::debug!(error = %e, "social poll: notifications fetch failed"),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    fn note(kind: &str, at: DateTime<Utc>, read: bool) -> Notification {
        Notification {
            id: Uuid::now_v7(),
            kind: kind.to_string(),
            actor_handle: Some("Wingman".to_string()),
            payload: serde_json::json!({}),
            created_at: at,
            read_at: read.then_some(at),
        }
    }

    #[test]
    fn only_unread_rows_newer_than_the_watermark_toast() {
        let w = Utc::now();
        let items = vec![
            note("friend_request", w - ChronoDuration::minutes(5), false),
            note("friend_request", w + ChronoDuration::seconds(5), true),
            note("friend_accepted", w + ChronoDuration::seconds(10), false),
        ];
        match plan_toasts(&items, w, false) {
            ToastPlan::Each(list) => {
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].0, "Friend request accepted");
                assert!(list[0].1.contains("@Wingman"));
            }
            other => panic!("expected one toast, got {other:?}"),
        }
    }

    #[test]
    fn in_game_quiet_holds_instead_of_toasting() {
        let w = Utc::now();
        let items = vec![note(
            "friend_request",
            w + ChronoDuration::seconds(1),
            false,
        )];
        assert_eq!(plan_toasts(&items, w, true), ToastPlan::Hold);
        // Nothing new means nothing to hold either.
        assert_eq!(plan_toasts(&[], w, true), ToastPlan::Nothing);
    }

    #[test]
    fn a_burst_collapses_into_one_summary() {
        let w = Utc::now();
        let items: Vec<Notification> = (1..=5)
            .map(|i| note("friend_request", w + ChronoDuration::seconds(i), false))
            .collect();
        assert_eq!(plan_toasts(&items, w, false), ToastPlan::Summary(5));
    }

    #[test]
    fn unknown_kinds_do_not_toast() {
        let w = Utc::now();
        let items = vec![note("lfg_join", w + ChronoDuration::seconds(1), false)];
        assert_eq!(plan_toasts(&items, w, false), ToastPlan::Nothing);
    }

    #[test]
    fn path_segments_cannot_escape() {
        assert_eq!(seg("Wing_man-7"), "Wing_man-7");
        assert_eq!(seg("../admin"), "%2E%2E%2Fadmin");
        assert_eq!(seg(" a b "), "a%20b");
    }

    #[test]
    fn dtos_round_trip_the_server_shape() {
        let json = serde_json::json!({
            "friends": [{ "handle": "Bob", "since": "2026-09-01T00:00:00Z", "rsi_verified": true }],
            "incoming": [{
                "id": "0199a000-0000-7000-8000-000000000001",
                "requester_handle": "Carol", "recipient_handle": "Alice",
                "status": "pending", "created_at": "2026-09-01T00:00:00Z", "responded_at": null
            }],
            "outgoing": [],
            "friend_request_policy": "everyone"
        });
        let parsed: FriendsResponse = serde_json::from_value(json).unwrap();
        assert!(parsed.friends[0].rsi_verified);
        assert_eq!(parsed.incoming[0].requester_handle, "Carol");
    }

    #[test]
    fn unpaired_config_is_refused() {
        let cfg = crate::config::Config::default();
        assert!(matches!(
            SocialClient::from_config(&cfg),
            Err(SocialClientError::NotPaired)
        ));
    }
}
