//! The realtime gateway and presence routes (social phase 3). See
//! `presence.rs` for what is shared, with whom, and what is kept.
//!
//! - `GET /v1/ws` upgrades to a WebSocket. The tray reports its presence
//!   there and is pushed friends' presence and a nudge when a
//!   notification arrives.
//! - `PUT` / `DELETE /v1/me/presence` are the same report over HTTP, for
//!   a client without the socket.
//! - `GET /v1/me/friends/presence` is friends' presence, for the web,
//!   which polls rather than holding a socket.
//! - `GET` / `PUT /v1/me/presence/settings` is the server-side gate.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::notifications::{Notification, NotificationError, NotificationKind, NotificationStore};
use crate::presence::{
    clean_system, ClientMessage, FriendPresence, PresenceHub, PresenceLevel, PresenceSettingsStore,
    PresenceUpdate, ServerMessage,
};
use crate::social::SocialStore;
use crate::social_routes::caller;
use crate::users::UserStore;
use async_trait::async_trait;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// A client message larger than this is not a presence report.
const MAX_CLIENT_MESSAGE: usize = 4 * 1024;

pub fn routes() -> Router {
    Router::new()
        .route("/v1/ws", get(gateway))
        .route("/v1/me/presence", put(report).delete(go_offline))
        .route(
            "/v1/me/presence/settings",
            get(get_settings).put(put_settings),
        )
        .route("/v1/me/friends/presence", get(friends_presence))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PresenceSettings {
    pub level: PresenceLevel,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FriendsPresenceResponse {
    pub friends: Vec<FriendPresence>,
}

fn err(status: StatusCode, code: &str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

// -- The shared core ----------------------------------------------------------

/// Tell `handle`'s friends what they may now see of them.
async fn fan_out(
    hub: &PresenceHub,
    social: &dyn SocialStore,
    handle: &str,
    presence: FriendPresence,
) {
    match social.list_friends(handle).await {
        Ok(friends) => {
            for f in friends {
                hub.send_to(&f.handle, ServerMessage::Presence(presence.clone()));
            }
        }
        Err(e) => tracing::warn!(error = %e, "presence fan-out: list_friends failed"),
    }
}

/// Apply a report from `handle`, through the server-side gate.
async fn apply_report(
    hub: &PresenceHub,
    social: &dyn SocialStore,
    settings: &dyn PresenceSettingsStore,
    handle: &str,
    update: &PresenceUpdate,
    now: DateTime<Utc>,
) -> Result<(), Response> {
    let level = settings.level(handle).await.map_err(|e| {
        tracing::error!(error = %e, "presence level read failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
    })?;
    if level == PresenceLevel::Off {
        // The tray's gate is open and ours is not: keep nothing.
        if hub.clear(handle) {
            fan_out(hub, social, handle, FriendPresence::offline(handle)).await;
        }
        return Ok(());
    }
    // Store the system only when it may be shown; data we will not show
    // is not kept.
    let system = if level == PresenceLevel::System {
        clean_system(update.system.as_deref())
    } else {
        None
    };
    if hub.set(handle, update.state, system, now) {
        fan_out(hub, social, handle, hub.view(handle, level, now)).await;
    }
    Ok(())
}

async fn apply_offline(hub: &PresenceHub, social: &dyn SocialStore, handle: &str) {
    if hub.clear(handle) {
        fan_out(hub, social, handle, FriendPresence::offline(handle)).await;
    }
}

/// Friends' presence as `handle` may see it.
async fn friends_view(
    hub: &PresenceHub,
    social: &dyn SocialStore,
    settings: &dyn PresenceSettingsStore,
    handle: &str,
    now: DateTime<Utc>,
) -> Result<Vec<FriendPresence>, Response> {
    let friends: Vec<String> = social
        .list_friends(handle)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "friends presence: list_friends failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        })?
        .into_iter()
        .map(|f| f.handle)
        .collect();
    let levels = settings.levels(&friends).await.map_err(|e| {
        tracing::error!(error = %e, "friends presence: levels read failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
    })?;
    Ok(friends
        .iter()
        .map(|f| {
            let level = levels
                .get(&f.to_lowercase())
                .copied()
                .unwrap_or(PresenceLevel::Off);
            hub.view(f, level, now)
        })
        .collect())
}

// -- HTTP ---------------------------------------------------------------------

/// Report your presence. Kept only if your presence setting is not
/// `off`; the system only if it is `system`.
#[utoipa::path(
    put,
    path = "/v1/me/presence",
    tag = "social",
    operation_id = "social_report_presence",
    request_body = PresenceUpdate,
    responses((status = 204, description = "Reported (or ignored: presence is off)")),
    security(("bearer" = [])),
)]
pub async fn report(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(settings): Extension<Arc<dyn PresenceSettingsStore>>,
    Extension(hub): Extension<Arc<PresenceHub>>,
    Json(update): Json<PresenceUpdate>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match apply_report(
        &hub,
        social.as_ref(),
        settings.as_ref(),
        &me.claimed_handle,
        &update,
        Utc::now(),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(r) => r,
    }
}

/// Stop showing as present now, rather than when the report expires.
#[utoipa::path(
    delete,
    path = "/v1/me/presence",
    tag = "social",
    operation_id = "social_clear_presence",
    responses((status = 204, description = "Shown as offline")),
    security(("bearer" = [])),
)]
pub async fn go_offline(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(hub): Extension<Arc<PresenceHub>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    apply_offline(&hub, social.as_ref(), &me.claimed_handle).await;
    StatusCode::NO_CONTENT.into_response()
}

/// Your friends' presence. Anyone not sharing reads as offline.
#[utoipa::path(
    get,
    path = "/v1/me/friends/presence",
    tag = "social",
    operation_id = "social_friends_presence",
    responses((status = 200, description = "Friends' presence", body = FriendsPresenceResponse)),
    security(("bearer" = [])),
)]
pub async fn friends_presence(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(settings): Extension<Arc<dyn PresenceSettingsStore>>,
    Extension(hub): Extension<Arc<PresenceHub>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match friends_view(
        &hub,
        social.as_ref(),
        settings.as_ref(),
        &me.claimed_handle,
        Utc::now(),
    )
    .await
    {
        Ok(friends) => Json(FriendsPresenceResponse { friends }).into_response(),
        Err(r) => r,
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/presence/settings",
    tag = "social",
    operation_id = "social_get_presence_settings",
    responses((status = 200, description = "Your presence setting", body = PresenceSettings)),
    security(("bearer" = [])),
)]
pub async fn get_settings(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(settings): Extension<Arc<dyn PresenceSettingsStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match settings.level(&me.claimed_handle).await {
        Ok(level) => Json(PresenceSettings { level }).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "presence level read failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

/// Set how much of your presence friends see. Takes effect at once:
/// turning it off, or down from `system` to `status`, is pushed to
/// friends straight away rather than when the next report arrives.
#[utoipa::path(
    put,
    path = "/v1/me/presence/settings",
    tag = "social",
    operation_id = "social_put_presence_settings",
    request_body = PresenceSettings,
    responses((status = 200, description = "Saved, read back", body = PresenceSettings)),
    security(("bearer" = [])),
)]
pub async fn put_settings(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(settings): Extension<Arc<dyn PresenceSettingsStore>>,
    Extension(hub): Extension<Arc<PresenceHub>>,
    Json(body): Json<PresenceSettings>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let handle = me.claimed_handle.as_str();
    if let Err(e) = settings.set_level(handle, body.level).await {
        tracing::error!(error = %e, "presence level write failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }
    let stored = match settings.level(handle).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "presence level read-back failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    if stored == PresenceLevel::Off {
        apply_offline(&hub, social.as_ref(), handle).await;
    } else {
        let now = Utc::now();
        let view = hub.view(handle, stored, now);
        if view.state.is_some() {
            // Narrowing to `status` must also drop the kept system.
            if stored == PresenceLevel::Status {
                if let Some(state) = view.state {
                    hub.set(handle, state, None, now);
                }
            }
            fan_out(&hub, social.as_ref(), handle, view).await;
        }
    }
    Json(PresenceSettings { level: stored }).into_response()
}

// -- WebSocket ----------------------------------------------------------------

/// The realtime gateway. Authenticated like any other route (the tray
/// sends its bearer token on the upgrade request). On connect it sends
/// each friend's presence, then pushes changes and notification nudges;
/// the client sends presence reports, `offline`, or `ping`.
#[utoipa::path(
    get,
    path = "/v1/ws",
    tag = "social",
    operation_id = "social_gateway",
    responses(
        (status = 101, description = "Switching to the realtime gateway"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("bearer" = [])),
)]
pub async fn gateway(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(settings): Extension<Arc<dyn PresenceSettingsStore>>,
    Extension(hub): Extension<Arc<PresenceHub>>,
    ws: WebSocketUpgrade,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    ws.max_message_size(MAX_CLIENT_MESSAGE)
        .on_upgrade(move |socket| run_connection(socket, me.claimed_handle, hub, social, settings))
}

async fn run_connection(
    mut socket: WebSocket,
    handle: String,
    hub: Arc<PresenceHub>,
    social: Arc<dyn SocialStore>,
    settings: Arc<dyn PresenceSettingsStore>,
) {
    let (id, mut pushes) = hub.register(&handle);
    if let Ok(friends) = friends_view(
        &hub,
        social.as_ref(),
        settings.as_ref(),
        &handle,
        Utc::now(),
    )
    .await
    {
        for p in friends {
            if send(&mut socket, &ServerMessage::Presence(p))
                .await
                .is_err()
            {
                hub.unregister(&handle, id);
                return;
            }
        }
    }
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<ClientMessage>(text.as_str()) {
                        Ok(ClientMessage::Presence(update)) => {
                            let _ = apply_report(
                                &hub,
                                social.as_ref(),
                                settings.as_ref(),
                                &handle,
                                &update,
                                Utc::now(),
                            )
                            .await;
                        }
                        Ok(ClientMessage::Offline) => {
                            apply_offline(&hub, social.as_ref(), &handle).await;
                        }
                        Ok(ClientMessage::Ping) => {}
                        Err(_) => {
                            tracing::debug!("gateway: ignoring an unreadable client message");
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            push = pushes.recv() => match push {
                Some(msg) => {
                    if send(&mut socket, &msg).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
        }
    }
    // The last connection closing is the tray going away.
    if hub.unregister(&handle, id) {
        apply_offline(&hub, social.as_ref(), &handle).await;
    }
}

async fn send(socket: &mut WebSocket, msg: &ServerMessage) -> Result<(), axum::Error> {
    let text = serde_json::to_string(msg).expect("server messages always serialise");
    socket.send(Message::Text(text.into())).await
}

/// Forget reports nobody has refreshed, and tell friends. Presence is
/// read through `view`, which already treats a stale report as
/// offline; this is what makes the server actually let go of it.
pub fn spawn_sweep_loop(hub: Arc<PresenceHub>, social: Arc<dyn SocialStore>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            for handle in hub.sweep(Utc::now()) {
                fan_out(
                    &hub,
                    social.as_ref(),
                    &handle,
                    FriendPresence::offline(&handle),
                )
                .await;
            }
        }
    });
}

// -- Notification nudges ------------------------------------------------------

/// A `NotificationStore` that, on each new notification, nudges the
/// recipient's open connections to fetch their inbox. Everything else
/// passes straight through.
pub struct PushingNotificationStore {
    inner: Arc<dyn NotificationStore>,
    hub: Arc<PresenceHub>,
}

impl PushingNotificationStore {
    pub fn new(inner: Arc<dyn NotificationStore>, hub: Arc<PresenceHub>) -> Self {
        Self { inner, hub }
    }
}

#[async_trait]
impl NotificationStore for PushingNotificationStore {
    async fn create(
        &self,
        recipient: &str,
        kind: NotificationKind,
        actor: Option<&str>,
        payload: serde_json::Value,
    ) -> Result<Notification, NotificationError> {
        let n = self.inner.create(recipient, kind, actor, payload).await?;
        self.hub.send_to(recipient, ServerMessage::Notification);
        Ok(n)
    }

    async fn list(
        &self,
        recipient: &str,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Notification>, NotificationError> {
        self.inner.list(recipient, since, limit).await
    }

    async fn unread_count(&self, recipient: &str) -> Result<i64, NotificationError> {
        self.inner.unread_count(recipient).await
    }

    async fn mark_read(&self, recipient: &str, ids: &[Uuid]) -> Result<u64, NotificationError> {
        self.inner.mark_read(recipient, ids).await
    }

    async fn mark_all_read(&self, recipient: &str) -> Result<u64, NotificationError> {
        self.inner.mark_all_read(recipient).await
    }

    async fn delete_from_actor(
        &self,
        recipient: &str,
        actor: &str,
    ) -> Result<u64, NotificationError> {
        self.inner.delete_from_actor(recipient, actor).await
    }

    async fn purge_older_than(&self, before: DateTime<Utc>) -> Result<u64, NotificationError> {
        self.inner.purge_older_than(before).await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::TokenIssuer;
    use crate::notifications::test_support::MemoryNotificationStore;
    use crate::presence::test_support::MemoryPresenceSettingsStore;
    use crate::social::test_support::MemorySocialStore;
    use crate::users::hash_password;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use tower::ServiceExt;

    struct Fixture {
        app: Router,
        issuer: TokenIssuer,
        users: Arc<MemoryUserStore>,
        social: Arc<MemorySocialStore>,
        hub: Arc<PresenceHub>,
        notes: Arc<dyn NotificationStore>,
    }

    fn fixture() -> Fixture {
        let users = Arc::new(MemoryUserStore::new());
        let social = Arc::new(MemorySocialStore::new());
        let hub = Arc::new(PresenceHub::new());
        let (issuer, verifier) = fresh_pair();
        let users_dyn: Arc<dyn UserStore> = users.clone();
        let social_dyn: Arc<dyn SocialStore> = social.clone();
        let settings: Arc<dyn PresenceSettingsStore> = Arc::new(MemoryPresenceSettingsStore::new());
        let notes: Arc<dyn NotificationStore> = Arc::new(PushingNotificationStore::new(
            Arc::new(MemoryNotificationStore::new()),
            hub.clone(),
        ));
        let app = routes()
            .layer(Extension(users_dyn))
            .layer(Extension(social_dyn))
            .layer(Extension(settings))
            .layer(Extension(hub.clone()))
            .layer(Extension(Arc::new(verifier)));
        Fixture {
            app,
            issuer,
            users,
            social,
            hub,
            notes,
        }
    }

    impl Fixture {
        async fn user(&self, handle: &str) -> String {
            let phc = hash_password("password-123-abcdef").unwrap();
            let u = self
                .users
                .create(&format!("{handle}@example.com"), &phc, handle)
                .await
                .unwrap();
            self.issuer.sign_user(&u.id.to_string(), handle).unwrap()
        }

        async fn call(
            &self,
            method: &str,
            uri: &str,
            token: &str,
            body: Option<serde_json::Value>,
        ) -> (StatusCode, serde_json::Value) {
            let mut req = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"));
            let body = match body {
                Some(v) => {
                    req = req.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let resp = self
                .app
                .clone()
                .oneshot(req.body(body).unwrap())
                .await
                .unwrap();
            let status = resp.status();
            let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
            )
        }

        async fn level(&self, token: &str, level: &str) {
            let (s, v) = self
                .call(
                    "PUT",
                    "/v1/me/presence/settings",
                    token,
                    Some(serde_json::json!({ "level": level })),
                )
                .await;
            assert_eq!(s, StatusCode::OK);
            assert_eq!(v["level"], level);
        }

        async fn report(&self, token: &str, state: &str, system: Option<&str>) {
            let (s, _) = self
                .call(
                    "PUT",
                    "/v1/me/presence",
                    token,
                    Some(serde_json::json!({ "state": state, "system": system })),
                )
                .await;
            assert_eq!(s, StatusCode::NO_CONTENT);
        }

        async fn friends_presence(&self, token: &str) -> serde_json::Value {
            let (s, v) = self
                .call("GET", "/v1/me/friends/presence", token, None)
                .await;
            assert_eq!(s, StatusCode::OK);
            v["friends"].clone()
        }
    }

    #[tokio::test]
    async fn presence_is_off_until_the_owner_turns_it_on() {
        let f = fixture();
        let alice = f.user("Alice").await;
        let bob = f.user("Bob").await;
        f.social.add_friendship("Alice", "Bob").await.unwrap();

        let (s, v) = f
            .call("GET", "/v1/me/presence/settings", &alice, None)
            .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["level"], "off");

        // The tray's gate is open, ours is not: nothing is kept.
        f.report(&alice, "in_game", Some("Stanton")).await;
        assert_eq!(
            f.friends_presence(&bob).await[0]["state"],
            serde_json::Value::Null
        );
        assert!(!f.hub.clear("Alice"), "nothing was kept in memory");
    }

    #[tokio::test]
    async fn the_system_is_a_second_opt_in() {
        let f = fixture();
        let alice = f.user("Alice").await;
        let bob = f.user("Bob").await;
        let carol = f.user("Carol").await;
        f.social.add_friendship("Alice", "Bob").await.unwrap();

        f.level(&alice, "status").await;
        f.report(&alice, "in_quantum", Some("Pyro")).await;
        let seen = f.friends_presence(&bob).await;
        assert_eq!(seen[0]["handle"], "Alice");
        assert_eq!(seen[0]["state"], "in_quantum");
        assert_eq!(seen[0]["system"], serde_json::Value::Null);

        f.level(&alice, "system").await;
        f.report(&alice, "in_game", Some("Pyro")).await;
        assert_eq!(f.friends_presence(&bob).await[0]["system"], "Pyro");

        // Not a friend: not listed at all.
        assert_eq!(f.friends_presence(&carol).await, serde_json::json!([]));

        // Narrowing back to status drops the system from memory too.
        f.level(&alice, "status").await;
        f.level(&alice, "system").await;
        assert_eq!(
            f.friends_presence(&bob).await[0]["system"],
            serde_json::Value::Null,
            "the system was forgotten, not just hidden"
        );

        f.level(&alice, "off").await;
        assert_eq!(
            f.friends_presence(&bob).await[0]["state"],
            serde_json::Value::Null
        );
        assert!(!f.hub.clear("Alice"), "off forgets the report");
    }

    #[tokio::test]
    async fn an_unfit_system_name_is_dropped_not_shown() {
        let f = fixture();
        let alice = f.user("Alice").await;
        let bob = f.user("Bob").await;
        f.social.add_friendship("Alice", "Bob").await.unwrap();
        f.level(&alice, "system").await;
        f.report(&alice, "in_game", Some("<img src=x>")).await;
        let seen = f.friends_presence(&bob).await;
        assert_eq!(seen[0]["state"], "in_game");
        assert_eq!(seen[0]["system"], serde_json::Value::Null);
    }

    type Ws = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// The next pushed message as JSON, failing after 5s.
    async fn next_json(ws: &mut Ws) -> serde_json::Value {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("a message within 5s")
            .unwrap()
            .unwrap();
        serde_json::from_str(msg.to_text().unwrap()).unwrap()
    }

    /// The gateway over a real socket: auth on the upgrade, the snapshot
    /// on connect, a friend's report and a notification pushed, and the
    /// last connection closing showing its owner as offline.
    #[tokio::test]
    async fn the_gateway_pushes_presence_and_notification_nudges() {
        let f = fixture();
        let alice = f.user("Alice").await;
        let bob = f.user("Bob").await;
        f.social.add_friendship("Alice", "Bob").await.unwrap();
        f.level(&alice, "system").await;
        f.level(&bob, "status").await;
        f.report(&alice, "online", None).await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = f.app.clone();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let connect = |token: String| async move {
            let mut req = format!("ws://{addr}/v1/ws").into_client_request().unwrap();
            req.headers_mut()
                .insert("authorization", format!("Bearer {token}").parse().unwrap());
            tokio_tungstenite::connect_async(req)
                .await
                .map(|(ws, _)| ws)
        };
        assert!(
            connect("not-a-token".into()).await.is_err(),
            "the upgrade needs auth"
        );

        let mut bob_ws = connect(bob.clone()).await.unwrap();
        let snapshot = next_json(&mut bob_ws).await;
        assert_eq!(snapshot["type"], "presence");
        assert_eq!(snapshot["handle"], "Alice");
        assert_eq!(snapshot["state"], "online");

        // Alice reports over her own socket; Bob is told.
        let mut alice_ws = connect(alice.clone()).await.unwrap();
        let _alice_snapshot = next_json(&mut alice_ws).await;
        alice_ws
            .send(WsMessage::Text(
                r#"{"type":"presence","state":"in_game","system":"Stanton"}"#.into(),
            ))
            .await
            .unwrap();
        let pushed = next_json(&mut bob_ws).await;
        assert_eq!(pushed["state"], "in_game");
        assert_eq!(pushed["system"], "Stanton");

        // A notification for Bob nudges his connection.
        f.notes
            .create(
                "Bob",
                NotificationKind::Salute,
                Some("Alice"),
                serde_json::json!({}),
            )
            .await
            .unwrap();
        assert_eq!(next_json(&mut bob_ws).await["type"], "notification");

        // Alice's last connection closes: Bob sees her go offline.
        alice_ws.close(None).await.unwrap();
        let gone = next_json(&mut bob_ws).await;
        assert_eq!(gone["handle"], "Alice");
        assert_eq!(gone["state"], serde_json::Value::Null);
    }
}
