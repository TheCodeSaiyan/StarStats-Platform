//! Chat access routes (social phase 6). See `chat.rs`.
//!
//! - `GET /v1/me/chat` says whether the caller can chat, and if not, why.
//! - `POST /v1/me/chat/age-declaration` records the age declaration.
//! - `POST /v1/me/matrix/login-token` mints the short-lived token a client
//!   exchanges with Synapse for a Matrix session.
//!
//! Without chat configured (no matrix login key), status says so and the
//! token route answers 503 `chat_unavailable`.

use crate::account_restrictions::{AccountRestrictionStore, Capability};
use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::chat::{
    declared_for_chat, ChatAccessStore, ChatLaunch, MatrixLoginSigner, CHAT_MIN_AGE,
};
use crate::social_routes::caller;
use crate::staff_roles::StaffRoleStore;
use crate::users::{User, UserStore};
use axum::extract::Path;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

pub fn routes() -> Router {
    Router::new()
        .route("/v1/me/chat", get(chat_status))
        .route("/v1/me/chat/age-declaration", post(declare_age))
        .route("/v1/me/matrix/login-token", post(login_token))
        .route("/v1/me/chat/dm/{handle}", post(open_dm))
        .route("/v1/me/chat/rooms", get(my_rooms))
}

/// Mints are cheap for us and each one can open a Matrix session, so a
/// client that loops is capped: a burst of 10, one more every 6 s.
pub struct ChatTokenLimiter(crate::salutes::SaluteRateLimiter);

impl Default for ChatTokenLimiter {
    fn default() -> Self {
        Self(crate::salutes::SaluteRateLimiter::with_rate(
            10.0,
            1.0 / 6.0,
        ))
    }
}

impl ChatTokenLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check(&self, handle: &str) -> bool {
        self.0.check(handle)
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChatStatus {
    /// Chat is switched on for this service.
    pub available: bool,
    /// Every gate below is met: the caller can get a login token.
    pub can_chat: bool,
    pub rsi_verified: bool,
    /// The caller has declared they meet `minimum_age`.
    pub age_declared: bool,
    pub minimum_age: i16,
    /// A moderator has restricted the caller from chat.
    pub restricted: bool,
    /// The caller's Matrix user ID, when chat is available.
    pub user_id: Option<String>,
    /// The homeserver's client API base URL, when chat is available.
    pub homeserver_url: Option<String>,
    /// Clients should offer a way into chat: it is available and its
    /// launch switch (`STARSTATS_CHAT_ENABLED`) includes the caller. Only
    /// `GET /v1/me/chat` works this out; other responses say false.
    #[serde(default)]
    pub offered: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AgeDeclaration {
    /// The minimum age the player declares they meet. Must be the current
    /// minimum, so a client showing an out-of-date number cannot record it.
    pub minimum_age: i16,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MatrixLoginToken {
    /// Exchange at the homeserver with `org.matrix.login.jwt`.
    pub token: String,
    pub user_id: String,
    pub homeserver_url: String,
    /// Seconds until the token expires.
    pub expires_in: i64,
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

fn internal(what: &str, e: impl std::fmt::Display) -> Response {
    tracing::error!(error = %e, what, "chat access failed");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
}

/// Every gate, read once. Restrictions fail closed: if they cannot be
/// read, the answer is "restricted", never a token.
async fn status_for(
    me: &User,
    chat: &dyn ChatAccessStore,
    restrictions: &dyn AccountRestrictionStore,
    signer: &Option<MatrixLoginSigner>,
) -> Result<ChatStatus, Response> {
    let declaration = chat
        .age_declaration(&me.claimed_handle)
        .await
        .map_err(|e| internal("age_declaration", e))?;
    let restricted = match restrictions.effective(me.id).await {
        Ok(r) => r.is_some_and(|r| r.blocks(Capability::Chat)),
        Err(e) => {
            tracing::warn!(error = %e, "chat: restriction lookup failed; treating as restricted");
            true
        }
    };
    let rsi_verified = me.rsi_verified_at.is_some();
    let age_declared = declared_for_chat(declaration);
    let available = signer.is_some();
    Ok(ChatStatus {
        available,
        can_chat: available && rsi_verified && age_declared && !restricted,
        rsi_verified,
        age_declared,
        minimum_age: CHAT_MIN_AGE,
        restricted,
        user_id: signer.as_ref().map(|s| s.user_id(&me.claimed_handle)),
        homeserver_url: signer.as_ref().map(|s| s.public_url.clone()),
        offered: false,
    })
}

/// Whether the caller can chat, and if not, which gate is closed.
#[utoipa::path(
    get,
    path = "/v1/me/chat",
    tag = "chat",
    operation_id = "chat_status",
    responses((status = 200, description = "Chat status", body = ChatStatus)),
    security(("bearer" = [])),
)]
pub async fn chat_status(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(chat): Extension<Arc<dyn ChatAccessStore>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Extension(signer): Extension<Arc<Option<MatrixLoginSigner>>>,
    Extension(launch): Extension<Arc<ChatLaunch>>,
    Extension(staff): Extension<Arc<dyn StaffRoleStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let mut status = match status_for(&me, chat.as_ref(), restrictions.as_ref(), &signer).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    // Staff are looked up only when it matters. A failed lookup offers
    // nothing: this is what a client shows, never what it may do.
    let is_staff = *launch == ChatLaunch::Staff
        && match staff.list_active_for_user(me.id).await {
            Ok(roles) => !roles.as_strings().is_empty(),
            Err(e) => {
                tracing::warn!(error = %e, "chat: staff lookup failed; not offering chat");
                false
            }
        };
    status.offered = status.available && launch.offers(is_staff);
    Json(status).into_response()
}

/// Declare you meet the minimum age for chat. Idempotent.
#[utoipa::path(
    post,
    path = "/v1/me/chat/age-declaration",
    tag = "chat",
    operation_id = "chat_declare_age",
    request_body = AgeDeclaration,
    responses(
        (status = 200, description = "Recorded; chat status after it", body = ChatStatus),
        (status = 400, description = "Not the current minimum age", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn declare_age(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(chat): Extension<Arc<dyn ChatAccessStore>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Extension(signer): Extension<Arc<Option<MatrixLoginSigner>>>,
    Json(body): Json<AgeDeclaration>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if body.minimum_age != CHAT_MIN_AGE {
        return err(StatusCode::BAD_REQUEST, "wrong_minimum_age");
    }
    if let Err(e) = chat.declare_age(&me.claimed_handle, CHAT_MIN_AGE).await {
        return internal("declare_age", e);
    }
    match status_for(&me, chat.as_ref(), restrictions.as_ref(), &signer).await {
        Ok(s) => Json(s).into_response(),
        Err(r) => r,
    }
}

/// A Matrix login token, valid for a minute. Needs a verified RSI handle,
/// the age declaration and no chat restriction.
#[utoipa::path(
    post,
    path = "/v1/me/matrix/login-token",
    tag = "chat",
    operation_id = "chat_login_token",
    responses(
        (status = 200, description = "A login token", body = MatrixLoginToken),
        (status = 403, description = "rsi_handle_not_verified, age_not_declared or chat_restricted", body = ApiErrorBody),
        (status = 429, description = "Too many tokens", body = ApiErrorBody),
        (status = 503, description = "Chat is not available", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn login_token(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(chat): Extension<Arc<dyn ChatAccessStore>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Extension(signer): Extension<Arc<Option<MatrixLoginSigner>>>,
    Extension(limiter): Extension<Arc<ChatTokenLimiter>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(s) = signer.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "chat_unavailable");
    };
    let status = match status_for(&me, chat.as_ref(), restrictions.as_ref(), &signer).await {
        Ok(st) => st,
        Err(r) => return r,
    };
    if !status.rsi_verified {
        return err(StatusCode::FORBIDDEN, "rsi_handle_not_verified");
    }
    if !status.age_declared {
        return err(StatusCode::FORBIDDEN, "age_not_declared");
    }
    if status.restricted {
        return err(StatusCode::FORBIDDEN, "chat_restricted");
    }
    if !limiter.check(&me.claimed_handle) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    }
    let now = Utc::now();
    match s.mint(&me.claimed_handle, now) {
        Ok((token, exp)) => Json(MatrixLoginToken {
            token,
            user_id: s.user_id(&me.claimed_handle),
            homeserver_url: s.public_url.clone(),
            expires_in: exp - now.timestamp(),
        })
        .into_response(),
        Err(e) => internal("mint", e),
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DmRoom {
    pub room_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MyChatRooms {
    pub rooms: Vec<crate::chat_rooms::MyChatRoom>,
}

/// The DM room with a friend, created on first request. Both must be able
/// to chat; the other player's reason for not being able to is not said.
#[utoipa::path(
    post,
    path = "/v1/me/chat/dm/{handle}",
    tag = "chat",
    operation_id = "chat_open_dm",
    params(("handle" = String, Path, description = "The friend to message")),
    responses(
        (status = 200, description = "The DM room", body = DmRoom),
        (status = 400, description = "Yourself, or an invalid handle", body = ApiErrorBody),
        (status = 403, description = "rsi_handle_not_verified, age_not_declared or chat_restricted", body = ApiErrorBody),
        (status = 404, description = "Not a friend", body = ApiErrorBody),
        (status = 409, description = "They cannot be messaged", body = ApiErrorBody),
        (status = 503, description = "Chat is not available", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn open_dm(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn crate::social::SocialStore>>,
    Extension(chat): Extension<Arc<dyn ChatAccessStore>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Extension(signer): Extension<Arc<Option<MatrixLoginSigner>>>,
    Extension(rooms): Extension<Arc<Option<crate::chat_rooms::ChatRooms>>>,
    Path(handle): Path<String>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(rooms) = rooms.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "chat_unavailable");
    };
    let status = match status_for(&me, chat.as_ref(), restrictions.as_ref(), &signer).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !status.rsi_verified {
        return err(StatusCode::FORBIDDEN, "rsi_handle_not_verified");
    }
    if !status.age_declared {
        return err(StatusCode::FORBIDDEN, "age_not_declared");
    }
    if status.restricted {
        return err(StatusCode::FORBIDDEN, "chat_restricted");
    }
    let them = match crate::social_routes::target(&handle, users.as_ref()).await {
        Ok(u) => u,
        Err(r) if r.status() == StatusCode::BAD_REQUEST => return r,
        Err(_) => return err(StatusCode::NOT_FOUND, "not_friends"),
    };
    if them.id == me.id {
        return err(StatusCode::BAD_REQUEST, "cannot_message_self");
    }
    match social
        .are_friends(&me.claimed_handle, &them.claimed_handle)
        .await
    {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::NOT_FOUND, "not_friends"),
        Err(e) => return internal("are_friends", e),
    }
    // Their restriction fails closed too, and is not named: "cannot be
    // messaged" says nothing about why.
    let them_restricted = match restrictions.effective(them.id).await {
        Ok(r) => r.is_some_and(|r| r.blocks(Capability::Chat)),
        Err(_) => true,
    };
    if them_restricted {
        return err(StatusCode::CONFLICT, "cannot_message");
    }
    match rooms.dm(&me.claimed_handle, &them.claimed_handle).await {
        Ok(room_id) => Json(DmRoom { room_id }).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "chat: opening a DM failed");
            err(StatusCode::SERVICE_UNAVAILABLE, "chat_unavailable")
        }
    }
}

/// The chat rooms the API put you in, with what each is for: a crew's LFG
/// post, or the other player in a DM. Our screens need this because rooms
/// carry no name.
#[utoipa::path(
    get,
    path = "/v1/me/chat/rooms",
    tag = "chat",
    operation_id = "chat_my_rooms",
    responses((status = 200, description = "Your chat rooms", body = MyChatRooms)),
    security(("bearer" = [])),
)]
pub async fn my_rooms(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(rooms): Extension<Arc<Option<crate::chat_rooms::ChatRooms>>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(rooms) = rooms.as_ref() else {
        return Json(MyChatRooms { rooms: Vec::new() }).into_response();
    };
    match rooms.store.rooms_of(&me.claimed_handle).await {
        Ok(rooms) => Json(MyChatRooms { rooms }).into_response(),
        Err(e) => internal("rooms_of", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_restrictions::test_support::MemoryAccountRestrictionStore;
    use crate::account_restrictions::Restriction;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::TokenIssuer;
    use crate::chat::test_support::{signer, MemoryChatAccessStore};
    use crate::users::hash_password;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    struct Fixture {
        app: Router,
        issuer: TokenIssuer,
        users: Arc<MemoryUserStore>,
        restrictions: Arc<MemoryAccountRestrictionStore>,
        staff: Arc<crate::staff_roles::test_support::MemoryStaffRoleStore>,
        decoding: jsonwebtoken::DecodingKey,
    }

    fn fixture_with(chat_on: bool) -> Fixture {
        fixture_launch(chat_on, ChatLaunch::On)
    }

    fn fixture_launch(chat_on: bool, launch: ChatLaunch) -> Fixture {
        let users = Arc::new(MemoryUserStore::new());
        let staff = Arc::new(crate::staff_roles::test_support::MemoryStaffRoleStore::new());
        let restrictions = Arc::new(MemoryAccountRestrictionStore::new());
        let (issuer, verifier) = fresh_pair();
        let (s, decoding) = signer();
        let signer: Arc<Option<MatrixLoginSigner>> = Arc::new(chat_on.then_some(s));
        let app = routes()
            .layer(Extension(users.clone() as Arc<dyn UserStore>))
            .layer(Extension(
                Arc::new(MemoryChatAccessStore::new()) as Arc<dyn ChatAccessStore>
            ))
            .layer(Extension(
                restrictions.clone() as Arc<dyn AccountRestrictionStore>
            ))
            .layer(Extension(signer))
            .layer(Extension(Arc::new(ChatTokenLimiter::new())))
            .layer(Extension(Arc::new(launch)))
            .layer(Extension(staff.clone() as Arc<dyn StaffRoleStore>))
            .layer(Extension(Arc::new(verifier)));
        Fixture {
            app,
            issuer,
            users,
            restrictions,
            staff,
            decoding,
        }
    }

    impl Fixture {
        async fn user(&self, handle: &str, verified: bool) -> (String, uuid::Uuid) {
            let phc = hash_password("password-123-abcdef").unwrap();
            let u = self
                .users
                .create(&format!("{handle}@example.com"), &phc, handle)
                .await
                .unwrap();
            if verified {
                self.users.mark_rsi_verified(u.id).await.unwrap();
            }
            (
                self.issuer.sign_user(&u.id.to_string(), handle).unwrap(),
                u.id,
            )
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
    }

    fn chat_restriction() -> Restriction {
        Restriction {
            ingest_blocked: false,
            sharing_blocked: false,
            public_profile_blocked: false,
            submissions_blocked: false,
            chat_blocked: true,
            reason: "abusive messages".into(),
            restricted_by: "mod".into(),
            restricted_at: Utc::now(),
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn every_gate_must_be_open_for_a_token() {
        let f = fixture_with(true);
        let (unverified, _) = f.user("Rookie", false).await;
        let (s, v) = f
            .call("POST", "/v1/me/matrix/login-token", &unverified, None)
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "rsi_handle_not_verified");

        let (me, id) = f.user("Wing_Man", true).await;
        let (s, v) = f.call("POST", "/v1/me/matrix/login-token", &me, None).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "age_not_declared");

        let (s, v) = f
            .call(
                "POST",
                "/v1/me/chat/age-declaration",
                &me,
                Some(serde_json::json!({ "minimum_age": 16 })),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "only the current minimum");
        assert_eq!(v["error"], "wrong_minimum_age");
        let (s, v) = f
            .call(
                "POST",
                "/v1/me/chat/age-declaration",
                &me,
                Some(serde_json::json!({ "minimum_age": 18 })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["can_chat"], true);

        let (s, v) = f.call("POST", "/v1/me/matrix/login-token", &me, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["user_id"], "@wing_man:starstats.app");
        assert_eq!(v["expires_in"], 60);
        let mut val = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        val.set_audience(&["matrix"]);
        let claims = jsonwebtoken::decode::<serde_json::Value>(
            v["token"].as_str().unwrap(),
            &f.decoding,
            &val,
        )
        .unwrap()
        .claims;
        assert_eq!(claims["sub"], "wing_man");

        f.restrictions
            .upsert(id, &chat_restriction())
            .await
            .unwrap();
        let (s, v) = f.call("POST", "/v1/me/matrix/login-token", &me, None).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "chat_restricted");
        let (_, v) = f.call("GET", "/v1/me/chat", &me, None).await;
        assert_eq!(v["restricted"], true);
        assert_eq!(v["can_chat"], false);
    }

    #[tokio::test]
    async fn a_restriction_on_something_else_does_not_stop_chat() {
        let f = fixture_with(true);
        let (me, id) = f.user("Wingman", true).await;
        f.call(
            "POST",
            "/v1/me/chat/age-declaration",
            &me,
            Some(serde_json::json!({ "minimum_age": 18 })),
        )
        .await;
        let mut r = chat_restriction();
        r.chat_blocked = false;
        r.submissions_blocked = true;
        f.restrictions.upsert(id, &r).await.unwrap();
        let (s, _) = f.call("POST", "/v1/me/matrix/login-token", &me, None).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn chat_is_offered_as_the_launch_switch_says() {
        use crate::staff_roles::StaffRole;
        async fn offered(f: &Fixture, token: &str) -> serde_json::Value {
            f.call("GET", "/v1/me/chat", token, None).await.1["offered"].clone()
        }
        for (launch, player, moderator) in [
            (ChatLaunch::Off, false, false),
            (ChatLaunch::Staff, false, true),
            (ChatLaunch::On, true, true),
        ] {
            let f = fixture_launch(true, launch);
            let (p, _) = f.user("Player", true).await;
            let (m, mod_id) = f.user("Mod", true).await;
            f.staff
                .grant(mod_id, StaffRole::Moderator, None, None)
                .await
                .unwrap();
            assert_eq!(offered(&f, &p).await, player, "{launch:?} player");
            assert_eq!(offered(&f, &m).await, moderator, "{launch:?} moderator");
        }
        // Switched on but with no homeserver, nothing is offered.
        let f = fixture_launch(false, ChatLaunch::On);
        let (p, _) = f.user("Player", true).await;
        assert_eq!(offered(&f, &p).await, false);
    }

    #[tokio::test]
    async fn without_chat_configured_status_says_so_and_no_token_is_minted() {
        let f = fixture_with(false);
        let (me, _) = f.user("Wingman", true).await;
        let (_, v) = f.call("GET", "/v1/me/chat", &me, None).await;
        assert_eq!(v["available"], false);
        assert_eq!(v["can_chat"], false);
        assert_eq!(v["homeserver_url"], serde_json::Value::Null);
        let (s, v) = f.call("POST", "/v1/me/matrix/login-token", &me, None).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["error"], "chat_unavailable");
    }

    fn dm_fixture() -> (
        Fixture,
        Arc<crate::social::test_support::MemorySocialStore>,
        Arc<crate::chat_rooms::test_support::RecordingMatrix>,
    ) {
        let mut f = fixture_with(true);
        let social = Arc::new(crate::social::test_support::MemorySocialStore::new());
        let (rooms, _store, matrix) = crate::chat_rooms::test_support::rooms();
        let rooms: Arc<Option<crate::chat_rooms::ChatRooms>> =
            Arc::new(Arc::try_unwrap(rooms).ok());
        f.app = f
            .app
            .layer(Extension(
                social.clone() as Arc<dyn crate::social::SocialStore>
            ))
            .layer(Extension(rooms));
        (f, social, matrix)
    }

    async fn ready(f: &Fixture, handle: &str) -> (String, uuid::Uuid) {
        let (t, id) = f.user(handle, true).await;
        f.call(
            "POST",
            "/v1/me/chat/age-declaration",
            &t,
            Some(serde_json::json!({ "minimum_age": 18 })),
        )
        .await;
        (t, id)
    }

    #[tokio::test]
    async fn a_dm_opens_only_between_friends_and_once() {
        let (f, social, matrix) = dm_fixture();
        let (alice, _) = ready(&f, "Alice").await;
        let (_bob, _) = ready(&f, "Bob").await;
        let (s, v) = f.call("POST", "/v1/me/chat/dm/bob", &alice, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
        assert_eq!(v["error"], "not_friends");

        crate::social::SocialStore::add_friendship(social.as_ref(), "Alice", "Bob")
            .await
            .unwrap();
        let (s, v) = f.call("POST", "/v1/me/chat/dm/BOB", &alice, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let room = v["room_id"].as_str().unwrap().to_string();
        let (_, v) = f.call("POST", "/v1/me/chat/dm/bob", &alice, None).await;
        assert_eq!(v["room_id"], room.as_str(), "the same room again");
        assert_eq!(
            matrix.calls(),
            vec![format!(
                "create {room} invite=@alice:starstats.app,@bob:starstats.app mod=- direct=true"
            )]
        );
        let (_, v) = f.call("GET", "/v1/me/chat/rooms", &alice, None).await;
        assert_eq!(v["rooms"][0]["kind"], "dm");
        assert_eq!(v["rooms"][0]["other_handle"], "bob");

        let (s, v) = f.call("POST", "/v1/me/chat/dm/alice", &alice, None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "cannot_message_self");
    }

    #[tokio::test]
    async fn a_dm_needs_the_caller_to_qualify_and_the_friend_unrestricted() {
        let (f, social, _) = dm_fixture();
        let (carol, _) = f.user("Carol", true).await;
        let (_, dave_id) = ready(&f, "Dave").await;
        crate::social::SocialStore::add_friendship(social.as_ref(), "Carol", "Dave")
            .await
            .unwrap();
        let (s, v) = f.call("POST", "/v1/me/chat/dm/dave", &carol, None).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "age_not_declared");

        f.call(
            "POST",
            "/v1/me/chat/age-declaration",
            &carol,
            Some(serde_json::json!({ "minimum_age": 18 })),
        )
        .await;
        f.restrictions
            .upsert(dave_id, &chat_restriction())
            .await
            .unwrap();
        let (s, v) = f.call("POST", "/v1/me/chat/dm/dave", &carol, None).await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(v["error"], "cannot_message", "without saying why");
    }
}
