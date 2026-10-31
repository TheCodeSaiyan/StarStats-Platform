//! Chat rooms (social phase 6): the API creates every Matrix room and
//! decides every membership, through its application service.
//!
//! The chat guard module in Synapse lets only the service account
//! (`@starstats:<server_name>`) create rooms or invite, and lets a player
//! join only a room they were invited to. So:
//!
//! - a **crew room** is created when the host accepts the first player,
//!   with the host and that player invited; each later acceptance invites
//!   one more, and a removal kicks them;
//! - a **DM** is created on request between two friends; unfriending or
//!   blocking kicks both;
//! - a chat restriction or an account deletion kicks the player from every
//!   room they were invited to.
//!
//! Rooms are encrypted from creation, never federated, carry no name or
//! topic (both are server-visible), and give the service account sole
//! admin. A crew host gets moderator power, enough to remove players.
//!
//! Membership changes are best-effort from the StarStats side: a Synapse
//! hiccup must not fail an LFG answer or a block. They are logged by room
//! and handle, never by content (there is none here to log).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ChatRoomError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("homeserver error: {0}")]
    Homeserver(String),
}

/// Crew rooms and DMs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomKind {
    Crew,
    Dm,
}

impl RoomKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crew => "crew",
            Self::Dm => "dm",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "crew" => Some(Self::Crew),
            "dm" => Some(Self::Dm),
            _ => None,
        }
    }
}

/// A room the caller is in, as our screens need it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct MyChatRoom {
    pub room_id: String,
    pub kind: RoomKind,
    /// The LFG post, for a crew room.
    pub post_id: Option<Uuid>,
    /// The other player, lowercased, for a DM.
    pub other_handle: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A DM pair in the order it is stored: lowercased, smaller first.
pub fn dm_pair(a: &str, b: &str) -> (String, String) {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

#[async_trait]
pub trait ChatRoomStore: Send + Sync + 'static {
    async fn crew_room(&self, post_id: Uuid) -> Result<Option<String>, ChatRoomError>;
    async fn dm_room(&self, a: &str, b: &str) -> Result<Option<String>, ChatRoomError>;
    async fn insert_room(
        &self,
        room_id: &str,
        kind: RoomKind,
        post_id: Option<Uuid>,
        pair: Option<(String, String)>,
    ) -> Result<(), ChatRoomError>;
    async fn add_member(&self, room_id: &str, handle: &str) -> Result<(), ChatRoomError>;
    async fn remove_member(&self, room_id: &str, handle: &str) -> Result<(), ChatRoomError>;
    async fn close_room(&self, room_id: &str) -> Result<(), ChatRoomError>;
    /// Open rooms `handle` is a member of, newest first.
    async fn rooms_of(&self, handle: &str) -> Result<Vec<MyChatRoom>, ChatRoomError>;
    /// Whether `handle` is in `room_id` (invited by the API and not since
    /// removed), and the room is open.
    async fn is_member(&self, room_id: &str, handle: &str) -> Result<bool, ChatRoomError>;
}

pub struct PostgresChatRoomStore {
    pool: PgPool,
}

impl PostgresChatRoomStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChatRoomStore for PostgresChatRoomStore {
    async fn crew_room(&self, post_id: Uuid) -> Result<Option<String>, ChatRoomError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT room_id FROM chat_rooms
             WHERE kind = 'crew' AND post_id = $1 AND closed_at IS NULL",
        )
        .bind(post_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(r,)| r))
    }

    async fn dm_room(&self, a: &str, b: &str) -> Result<Option<String>, ChatRoomError> {
        let (a, b) = dm_pair(a, b);
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT room_id FROM chat_rooms
             WHERE kind = 'dm' AND dm_a = $1 AND dm_b = $2 AND closed_at IS NULL",
        )
        .bind(a)
        .bind(b)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(r,)| r))
    }

    async fn insert_room(
        &self,
        room_id: &str,
        kind: RoomKind,
        post_id: Option<Uuid>,
        pair: Option<(String, String)>,
    ) -> Result<(), ChatRoomError> {
        let (a, b) = match pair {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        };
        sqlx::query(
            "INSERT INTO chat_rooms (room_id, kind, post_id, dm_a, dm_b) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(room_id)
        .bind(kind.as_str())
        .bind(post_id)
        .bind(a)
        .bind(b)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn add_member(&self, room_id: &str, handle: &str) -> Result<(), ChatRoomError> {
        sqlx::query(
            "INSERT INTO chat_room_members (room_id, handle) VALUES ($1, lower($2))
             ON CONFLICT (room_id, handle) DO NOTHING",
        )
        .bind(room_id)
        .bind(handle)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_member(&self, room_id: &str, handle: &str) -> Result<(), ChatRoomError> {
        sqlx::query("DELETE FROM chat_room_members WHERE room_id = $1 AND handle = lower($2)")
            .bind(room_id)
            .bind(handle)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn close_room(&self, room_id: &str) -> Result<(), ChatRoomError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE chat_rooms SET closed_at = now() WHERE room_id = $1 AND closed_at IS NULL",
        )
        .bind(room_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM chat_room_members WHERE room_id = $1")
            .bind(room_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn is_member(&self, room_id: &str, handle: &str) -> Result<bool, ChatRoomError> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM chat_room_members m
             JOIN chat_rooms r ON r.room_id = m.room_id
             WHERE m.room_id = $1 AND m.handle = lower($2) AND r.closed_at IS NULL",
        )
        .bind(room_id)
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    async fn rooms_of(&self, handle: &str) -> Result<Vec<MyChatRoom>, ChatRoomError> {
        let rows: Vec<(
            String,
            String,
            Option<Uuid>,
            Option<String>,
            Option<String>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT r.room_id, r.kind, r.post_id, r.dm_a, r.dm_b, r.created_at
                 FROM chat_rooms r
                 JOIN chat_room_members m ON m.room_id = r.room_id
                 WHERE m.handle = lower($1) AND r.closed_at IS NULL
                 ORDER BY r.created_at DESC",
        )
        .bind(handle)
        .fetch_all(&self.pool)
        .await?;
        let me = handle.to_ascii_lowercase();
        Ok(rows
            .into_iter()
            .filter_map(|(room_id, kind, post_id, a, b, created_at)| {
                let kind = RoomKind::parse(&kind)?;
                let other_handle = match (a, b) {
                    (Some(a), Some(b)) => Some(if a == me { b } else { a }),
                    _ => None,
                };
                Some(MyChatRoom {
                    room_id,
                    kind,
                    post_id,
                    other_handle,
                    created_at,
                })
            })
            .collect())
    }
}

/// What the API asks of the homeserver, as its application service.
#[async_trait]
pub trait MatrixRooms: Send + Sync + 'static {
    /// Create an encrypted, unfederated, nameless room, inviting `invite`.
    /// `moderator` gets power to remove players (a crew host).
    async fn create_room(
        &self,
        invite: &[String],
        moderator: Option<&str>,
        direct: bool,
    ) -> Result<String, ChatRoomError>;
    async fn invite(&self, room_id: &str, user_id: &str) -> Result<(), ChatRoomError>;
    async fn kick(&self, room_id: &str, user_id: &str, reason: &str) -> Result<(), ChatRoomError>;
    /// Deactivate a player's Matrix account and erase it (account
    /// deletion). Needs the service account to be a Synapse admin, which
    /// db-init sets. A deactivated ID can never be used again.
    async fn deactivate(&self, user_id: &str) -> Result<(), ChatRoomError>;
    /// Make sure the service account exists as a Synapse user, so db-init
    /// can mark it admin. Safe to repeat.
    async fn ensure_registered(&self) -> Result<(), ChatRoomError>;
}

/// Percent-encode for a URL path segment or query value. Matrix IDs are
/// ASCII; anything outside the unreserved set is encoded.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The room-creation request: everything a StarStats room is, in one body.
pub fn create_room_body(
    service_user: &str,
    invite: &[String],
    moderator: Option<&str>,
    direct: bool,
) -> serde_json::Value {
    let mut users = serde_json::Map::new();
    users.insert(service_user.to_string(), 100.into());
    if let Some(m) = moderator {
        users.insert(m.to_string(), 50.into());
    }
    serde_json::json!({
        "preset": "private_chat",
        "visibility": "private",
        "is_direct": direct,
        "invite": invite,
        "creation_content": { "m.federate": false },
        "initial_state": [
            {
                "type": "m.room.encryption",
                "state_key": "",
                "content": { "algorithm": "m.megolm.v1.aes-sha2" }
            },
            {
                "type": "m.room.history_visibility",
                "state_key": "",
                "content": { "history_visibility": "invited" }
            },
            {
                "type": "m.room.guest_access",
                "state_key": "",
                "content": { "guest_access": "forbidden" }
            }
        ],
        "power_level_content_override": {
            "users": users,
            "users_default": 0,
            "events_default": 0,
            "state_default": 100,
            "invite": 100,
            "kick": 50,
            "ban": 100,
            "redact": 50
        }
    })
}

pub struct HttpMatrixRooms {
    http: reqwest::Client,
    base: String,
    as_token: String,
    service_user: String,
}

impl HttpMatrixRooms {
    pub fn new(base: &str, as_token: String, service_user: String) -> Result<Self, ChatRoomError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| ChatRoomError::Homeserver(e.to_string()))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            as_token,
            service_user,
        })
    }

    async fn post(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ChatRoomError> {
        let url = format!("{}{}?user_id={}", self.base, path, enc(&self.service_user));
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.as_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChatRoomError::Homeserver(e.without_url().to_string()))?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        if !status.is_success() {
            // errcode only: the error text can echo request detail.
            let code = json["errcode"].as_str().unwrap_or("unknown");
            return Err(ChatRoomError::Homeserver(format!(
                "{status} {code} on {path}"
            )));
        }
        Ok(json)
    }
}

#[async_trait]
impl MatrixRooms for HttpMatrixRooms {
    async fn create_room(
        &self,
        invite: &[String],
        moderator: Option<&str>,
        direct: bool,
    ) -> Result<String, ChatRoomError> {
        let body = create_room_body(&self.service_user, invite, moderator, direct);
        let v = self.post("/_matrix/client/v3/createRoom", body).await?;
        v["room_id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| ChatRoomError::Homeserver("createRoom returned no room_id".into()))
    }

    async fn invite(&self, room_id: &str, user_id: &str) -> Result<(), ChatRoomError> {
        self.post(
            &format!("/_matrix/client/v3/rooms/{}/invite", enc(room_id)),
            serde_json::json!({ "user_id": user_id }),
        )
        .await
        .map(|_| ())
    }

    async fn kick(&self, room_id: &str, user_id: &str, reason: &str) -> Result<(), ChatRoomError> {
        self.post(
            &format!("/_matrix/client/v3/rooms/{}/kick", enc(room_id)),
            serde_json::json!({ "user_id": user_id, "reason": reason }),
        )
        .await
        .map(|_| ())
    }

    async fn deactivate(&self, user_id: &str) -> Result<(), ChatRoomError> {
        self.post(
            &format!("/_synapse/admin/v1/deactivate/{}", enc(user_id)),
            serde_json::json!({ "erase": true }),
        )
        .await
        .map(|_| ())
    }

    async fn ensure_registered(&self) -> Result<(), ChatRoomError> {
        let localpart = self
            .service_user
            .trim_start_matches('@')
            .split(':')
            .next()
            .unwrap_or_default()
            .to_string();
        let resp = self
            .http
            .post(format!("{}/_matrix/client/v3/register", self.base))
            .bearer_auth(&self.as_token)
            .json(&serde_json::json!({
                "type": "m.login.application_service",
                "username": localpart,
                "inhibit_login": true,
            }))
            .send()
            .await
            .map_err(|e| ChatRoomError::Homeserver(e.without_url().to_string()))?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        // Already registered is the usual answer after the first start.
        if status.is_success() || json["errcode"] == "M_USER_IN_USE" {
            return Ok(());
        }
        let code = json["errcode"].as_str().unwrap_or("unknown");
        Err(ChatRoomError::Homeserver(format!(
            "{status} {code} on register"
        )))
    }
}

/// Turns a player's handle into their chat (Matrix) localpart.
///
/// The two differ only after a handle rename: Synapse cannot rename a user
/// ID, so a renamed player keeps the chat account they had, pinned in
/// `users.chat_localpart` (migration 0082). Everything in this module that
/// names a player in chat (room membership rows, Matrix user IDs, DM
/// pairs) uses the localpart, never the handle.
#[async_trait]
pub trait ChatLocalparts: Send + Sync {
    async fn localpart(&self, handle: &str) -> Result<String, ChatRoomError>;
}

pub struct PostgresChatLocalparts {
    pool: PgPool,
}

impl PostgresChatLocalparts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChatLocalparts for PostgresChatLocalparts {
    async fn localpart(&self, handle: &str) -> Result<String, ChatRoomError> {
        let derived = crate::chat::localpart(handle);
        let row: Option<(Uuid, Option<String>)> = sqlx::query_as(
            "SELECT id, chat_localpart FROM users WHERE lower(claimed_handle) = lower($1)",
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        let Some((user_id, pinned)) = row else {
            // No account holds the handle (a deleted player, or a name
            // from an old row): the derived localpart is the best answer.
            return Ok(derived);
        };
        if let Some(pinned) = pinned {
            return Ok(pinned);
        }
        // Once a renamed player's old handle is released, a new account may
        // claim it, and its derived localpart is the renamed player's chat
        // account. Give the newcomer a distinct one, pinned so it never
        // changes again.
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE chat_localpart = $1 AND id <> $2)",
        )
        .bind(&derived)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        if !taken {
            return Ok(derived);
        }
        let own = distinct_localpart(&derived, user_id);
        sqlx::query(
            "UPDATE users SET chat_localpart = $2 WHERE id = $1 AND chat_localpart IS NULL",
        )
        .bind(user_id)
        .bind(&own)
        .execute(&self.pool)
        .await?;
        // Re-read: a concurrent request may have pinned first.
        let pinned: Option<String> =
            sqlx::query_scalar("SELECT chat_localpart FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(pinned.unwrap_or(own))
    }
}

/// A localpart for an account whose derived one belongs to someone else:
/// the derived one plus the start of the account id, which Synapse's
/// localpart alphabet allows.
pub fn distinct_localpart(derived: &str, user_id: Uuid) -> String {
    format!("{derived}-{}", &user_id.simple().to_string()[..8])
}

/// Rooms and memberships, kept in step with StarStats. Holds everything
/// the hooks need so a call site is one line.
///
/// Every method takes handles and resolves them to chat localparts first
/// (see [`ChatLocalparts`]), so callers never deal in localparts.
pub struct ChatRooms {
    pub store: Arc<dyn ChatRoomStore>,
    pub matrix: Arc<dyn MatrixRooms>,
    pub server_name: String,
    pub localparts: Arc<dyn ChatLocalparts>,
}

impl ChatRooms {
    /// The Matrix user ID for a localpart. Callers holding a handle use
    /// [`Self::user_id_for`].
    pub fn user_id(&self, localpart: &str) -> String {
        format!(
            "@{}:{}",
            crate::chat::localpart(localpart),
            self.server_name
        )
    }

    /// The player's chat localpart.
    pub async fn chat_id(&self, handle: &str) -> Result<String, ChatRoomError> {
        self.localparts.localpart(handle).await
    }

    /// The Matrix user ID of the player holding `handle`.
    pub async fn user_id_for(&self, handle: &str) -> Result<String, ChatRoomError> {
        Ok(self.user_id(&self.chat_id(handle).await?))
    }

    /// A player joined a crew: invite them, creating the room (with the
    /// host) on the first acceptance.
    pub async fn crew_member_added(
        &self,
        post_id: Uuid,
        host: &str,
        member: &str,
    ) -> Result<String, ChatRoomError> {
        let host = &self.chat_id(host).await?;
        let member = &self.chat_id(member).await?;
        if let Some(room) = self.store.crew_room(post_id).await? {
            self.matrix.invite(&room, &self.user_id(member)).await?;
            self.store.add_member(&room, member).await?;
            return Ok(room);
        }
        let host_id = self.user_id(host);
        let room = self
            .matrix
            .create_room(
                &[host_id.clone(), self.user_id(member)],
                Some(&host_id),
                false,
            )
            .await?;
        self.store
            .insert_room(&room, RoomKind::Crew, Some(post_id), None)
            .await?;
        self.store.add_member(&room, host).await?;
        self.store.add_member(&room, member).await?;
        Ok(room)
    }

    /// A player left a crew by the host's hand: remove them from its room.
    pub async fn crew_member_removed(
        &self,
        post_id: Uuid,
        member: &str,
    ) -> Result<(), ChatRoomError> {
        let member = &self.chat_id(member).await?;
        let Some(room) = self.store.crew_room(post_id).await? else {
            return Ok(());
        };
        self.matrix
            .kick(&room, &self.user_id(member), "Removed from the crew")
            .await?;
        self.store.remove_member(&room, member).await
    }

    /// The DM between two players, created on first request.
    pub async fn dm(&self, me: &str, them: &str) -> Result<String, ChatRoomError> {
        let me = &self.chat_id(me).await?;
        let them = &self.chat_id(them).await?;
        if let Some(room) = self.store.dm_room(me, them).await? {
            return Ok(room);
        }
        let room = self
            .matrix
            .create_room(&[self.user_id(me), self.user_id(them)], None, true)
            .await?;
        self.store
            .insert_room(&room, RoomKind::Dm, None, Some(dm_pair(me, them)))
            .await?;
        self.store.add_member(&room, me).await?;
        self.store.add_member(&room, them).await?;
        Ok(room)
    }

    /// Unfriend or block: both leave the DM, and it is closed. Becoming
    /// friends again starts a new one.
    pub async fn end_dm(&self, a: &str, b: &str) -> Result<(), ChatRoomError> {
        let a = &self.chat_id(a).await?;
        let b = &self.chat_id(b).await?;
        let Some(room) = self.store.dm_room(a, b).await? else {
            return Ok(());
        };
        for h in [a, b] {
            if let Err(e) = self
                .matrix
                .kick(&room, &self.user_id(h), "No longer friends")
                .await
            {
                tracing::warn!(error = %e, room = %room, "chat: DM kick failed");
            }
        }
        self.store.close_room(&room).await
    }

    /// Account deletion: out of every room, then the Matrix account itself
    /// is deactivated and erased, as the chat DPIA says. The kicks come
    /// first so the rooms are left tidy even if deactivation fails.
    ///
    /// Takes the LOCALPART, resolved with [`Self::chat_id`] before the user
    /// row is deleted: a renamed player's chat account is named in that
    /// row, and resolving afterwards would close the wrong account and
    /// leave theirs active.
    pub async fn close_account(&self, localpart: &str) -> Result<(), ChatRoomError> {
        let kicked = self
            .remove_localpart_everywhere(localpart, "Account deleted")
            .await;
        self.matrix.deactivate(&self.user_id(localpart)).await?;
        kicked
    }

    /// Restriction or deletion: out of every room the API put them in.
    pub async fn remove_everywhere(&self, handle: &str, reason: &str) -> Result<(), ChatRoomError> {
        let localpart = self.chat_id(handle).await?;
        self.remove_localpart_everywhere(&localpart, reason).await
    }

    async fn remove_localpart_everywhere(
        &self,
        localpart: &str,
        reason: &str,
    ) -> Result<(), ChatRoomError> {
        for room in self.store.rooms_of(localpart).await? {
            if let Err(e) = self
                .matrix
                .kick(&room.room_id, &self.user_id(localpart), reason)
                .await
            {
                tracing::warn!(error = %e, room = %room.room_id, "chat: kick failed");
            }
            self.store.remove_member(&room.room_id, localpart).await?;
        }
        Ok(())
    }
}

/// The room service from a handler's optional extension: `None` when the
/// extension is not layered (tests) or chat is not configured.
pub fn from_ext(ext: &Option<axum::Extension<Arc<Option<ChatRooms>>>>) -> Option<&ChatRooms> {
    ext.as_ref()
        .and_then(|axum::Extension(c)| c.as_ref().as_ref())
}

/// Run a membership change without failing the caller: chat follows
/// StarStats, and a homeserver hiccup must not undo an LFG answer, a block
/// or a deletion. `what` names the change in the log.
pub async fn best_effort<F>(what: &'static str, fut: F)
where
    F: std::future::Future<Output = Result<(), ChatRoomError>>,
{
    if let Err(e) = fut.await {
        tracing::warn!(error = %e, what, "chat: membership change failed");
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryChatRoomStore {
        rooms: Mutex<
            Vec<(
                String,
                RoomKind,
                Option<Uuid>,
                Option<(String, String)>,
                bool,
            )>,
        >,
        members: Mutex<Vec<(String, String)>>,
    }

    impl MemoryChatRoomStore {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn members_of(&self, room: &str) -> Vec<String> {
            let mut v: Vec<String> = self
                .members
                .lock()
                .unwrap()
                .iter()
                .filter(|(r, _)| r == room)
                .map(|(_, h)| h.clone())
                .collect();
            v.sort();
            v
        }
    }

    #[async_trait]
    impl ChatRoomStore for MemoryChatRoomStore {
        async fn crew_room(&self, post_id: Uuid) -> Result<Option<String>, ChatRoomError> {
            Ok(self
                .rooms
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.1 == RoomKind::Crew && r.2 == Some(post_id) && !r.4)
                .map(|r| r.0.clone()))
        }

        async fn dm_room(&self, a: &str, b: &str) -> Result<Option<String>, ChatRoomError> {
            let pair = dm_pair(a, b);
            Ok(self
                .rooms
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.1 == RoomKind::Dm && r.3.as_ref() == Some(&pair) && !r.4)
                .map(|r| r.0.clone()))
        }

        async fn insert_room(
            &self,
            room_id: &str,
            kind: RoomKind,
            post_id: Option<Uuid>,
            pair: Option<(String, String)>,
        ) -> Result<(), ChatRoomError> {
            self.rooms
                .lock()
                .unwrap()
                .push((room_id.into(), kind, post_id, pair, false));
            Ok(())
        }

        async fn add_member(&self, room_id: &str, handle: &str) -> Result<(), ChatRoomError> {
            let mut m = self.members.lock().unwrap();
            let h = handle.to_ascii_lowercase();
            if !m.iter().any(|(r, x)| r == room_id && *x == h) {
                m.push((room_id.into(), h));
            }
            Ok(())
        }

        async fn remove_member(&self, room_id: &str, handle: &str) -> Result<(), ChatRoomError> {
            let h = handle.to_ascii_lowercase();
            self.members
                .lock()
                .unwrap()
                .retain(|(r, x)| !(r == room_id && *x == h));
            Ok(())
        }

        async fn close_room(&self, room_id: &str) -> Result<(), ChatRoomError> {
            for r in self.rooms.lock().unwrap().iter_mut() {
                if r.0 == room_id {
                    r.4 = true;
                }
            }
            self.members.lock().unwrap().retain(|(r, _)| r != room_id);
            Ok(())
        }

        async fn is_member(&self, room_id: &str, handle: &str) -> Result<bool, ChatRoomError> {
            let h = handle.to_ascii_lowercase();
            let open = self
                .rooms
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.0 == room_id && !r.4);
            Ok(open
                && self
                    .members
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(r, x)| r == room_id && *x == h))
        }

        async fn rooms_of(&self, handle: &str) -> Result<Vec<MyChatRoom>, ChatRoomError> {
            let me = handle.to_ascii_lowercase();
            let members = self.members.lock().unwrap();
            Ok(self
                .rooms
                .lock()
                .unwrap()
                .iter()
                .filter(|r| !r.4 && members.iter().any(|(room, h)| *room == r.0 && *h == me))
                .map(|r| MyChatRoom {
                    room_id: r.0.clone(),
                    kind: r.1,
                    post_id: r.2,
                    other_handle: r
                        .3
                        .as_ref()
                        .map(|(a, b)| if *a == me { b.clone() } else { a.clone() }),
                    created_at: Utc::now(),
                })
                .collect())
        }
    }

    /// Records every call the API makes of the homeserver.
    #[derive(Default)]
    pub struct RecordingMatrix {
        pub calls: Mutex<Vec<String>>,
        next: Mutex<u32>,
    }

    impl RecordingMatrix {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MatrixRooms for RecordingMatrix {
        async fn create_room(
            &self,
            invite: &[String],
            moderator: Option<&str>,
            direct: bool,
        ) -> Result<String, ChatRoomError> {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            let room = format!("!room{n}:starstats.app");
            self.calls.lock().unwrap().push(format!(
                "create {room} invite={} mod={} direct={direct}",
                invite.join(","),
                moderator.unwrap_or("-")
            ));
            Ok(room)
        }

        async fn invite(&self, room_id: &str, user_id: &str) -> Result<(), ChatRoomError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("invite {room_id} {user_id}"));
            Ok(())
        }

        async fn kick(&self, room_id: &str, user_id: &str, _: &str) -> Result<(), ChatRoomError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("kick {room_id} {user_id}"));
            Ok(())
        }

        async fn deactivate(&self, user_id: &str) -> Result<(), ChatRoomError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("deactivate {user_id}"));
            Ok(())
        }

        async fn ensure_registered(&self) -> Result<(), ChatRoomError> {
            Ok(())
        }
    }

    pub fn rooms() -> (
        Arc<ChatRooms>,
        Arc<MemoryChatRoomStore>,
        Arc<RecordingMatrix>,
    ) {
        let store = Arc::new(MemoryChatRoomStore::new());
        let matrix = Arc::new(RecordingMatrix::new());
        let rooms = Arc::new(ChatRooms {
            store: store.clone(),
            matrix: matrix.clone(),
            server_name: "starstats.app".into(),
            localparts: Arc::new(MemoryChatLocalparts::default()),
        });
        (rooms, store, matrix)
    }

    /// Lower-cases a handle, as for a never-renamed player, unless a pin
    /// has been set for it.
    #[derive(Default)]
    pub struct MemoryChatLocalparts {
        pub pinned: Mutex<std::collections::HashMap<String, String>>,
    }

    #[async_trait]
    impl ChatLocalparts for MemoryChatLocalparts {
        async fn localpart(&self, handle: &str) -> Result<String, ChatRoomError> {
            Ok(self
                .pinned
                .lock()
                .unwrap()
                .get(&handle.to_lowercase())
                .cloned()
                .unwrap_or_else(|| crate::chat::localpart(handle)))
        }
    }
}

#[cfg(test)]
mod localpart_tests {
    use super::test_support::*;
    use super::*;

    #[tokio::test]
    async fn a_renamed_player_keeps_their_chat_account_in_a_new_dm() {
        let (rooms, store, matrix) = rooms();
        // Pin as a rename does: NewPilot's chat account is @oldpilot.
        let pins = Arc::new(MemoryChatLocalparts::default());
        pins.pinned
            .lock()
            .unwrap()
            .insert("newpilot".into(), "oldpilot".into());
        let rooms = ChatRooms {
            store: rooms.store.clone(),
            matrix: rooms.matrix.clone(),
            server_name: rooms.server_name.clone(),
            localparts: pins,
        };

        let room = rooms.dm("NewPilot", "Friend").await.unwrap();
        let members = store.members_of(&room);
        assert!(members.contains(&"oldpilot".to_string()), "{members:?}");
        assert!(!members.iter().any(|m| m == "newpilot"));
        assert!(
            matrix
                .calls()
                .iter()
                .any(|c| c.contains("@oldpilot:starstats.app")),
            "the room invites the account they had: {:?}",
            matrix.calls()
        );
        // The same DM is found again under the new handle.
        assert_eq!(rooms.dm("NewPilot", "Friend").await.unwrap(), room);
        assert_eq!(
            rooms.user_id_for("NewPilot").await.unwrap(),
            "@oldpilot:starstats.app"
        );
    }

    #[test]
    fn a_distinct_localpart_keeps_the_name_and_adds_the_account() {
        let id = Uuid::parse_str("0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(distinct_localpart("oldpilot", id), "oldpilot-01234567");
    }

    /// Against the real schema. Skipped without STARSTATS_TEST_DATABASE_URL.
    #[tokio::test]
    async fn postgres_a_newcomer_on_a_released_handle_gets_their_own_chat_account() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres localpart test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let tag = &Uuid::new_v4().simple().to_string()[..8];
        let released = format!("lpOld{tag}");
        let (renamed, newcomer) = (Uuid::new_v4(), Uuid::new_v4());
        // The renamed player kept the chat account named after `released`.
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, claimed_handle, chat_localpart) \
             VALUES ($1, $2, 'phc', $3, $4)",
        )
        .bind(renamed)
        .bind(format!("r{tag}@example.com"))
        .bind(format!("lpNew{tag}"))
        .bind(released.to_lowercase())
        .execute(&pool)
        .await
        .unwrap();
        // Later, someone else claims `released`.
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, claimed_handle) \
             VALUES ($1, $2, 'phc', $3)",
        )
        .bind(newcomer)
        .bind(format!("n{tag}@example.com"))
        .bind(&released)
        .execute(&pool)
        .await
        .unwrap();

        let lp = PostgresChatLocalparts::new(pool.clone());
        assert_eq!(
            lp.localpart(&format!("lpNew{tag}")).await.unwrap(),
            released.to_lowercase()
        );
        let theirs = lp.localpart(&released).await.unwrap();
        assert_eq!(
            theirs,
            distinct_localpart(&released.to_lowercase(), newcomer)
        );
        // Pinned, so it never changes again.
        assert_eq!(lp.localpart(&released).await.unwrap(), theirs);
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn a_room_is_encrypted_unfederated_nameless_and_ours() {
        let b = create_room_body(
            "@starstats:starstats.app",
            &["@host:starstats.app".into()],
            Some("@host:starstats.app"),
            false,
        );
        assert_eq!(b["creation_content"]["m.federate"], false);
        assert!(b.get("name").is_none() && b.get("topic").is_none());
        let state = b["initial_state"].as_array().unwrap();
        assert!(state.iter().any(|e| e["type"] == "m.room.encryption"
            && e["content"]["algorithm"] == "m.megolm.v1.aes-sha2"));
        let pl = &b["power_level_content_override"];
        assert_eq!(pl["users"]["@starstats:starstats.app"], 100);
        assert_eq!(pl["users"]["@host:starstats.app"], 50);
        assert_eq!(pl["kick"], 50, "the host can remove players");
        assert_eq!(pl["invite"], 100, "only the service invites");
        assert_eq!(pl["state_default"], 100, "only the service changes state");
    }

    #[test]
    fn ids_are_encoded_for_urls() {
        assert_eq!(enc("!abc:starstats.app"), "%21abc%3Astarstats.app");
        assert_eq!(
            enc("@wing_man:starstats.app"),
            "%40wing_man%3Astarstats.app"
        );
    }

    #[test]
    fn a_dm_pair_is_one_room_whichever_way_round() {
        assert_eq!(dm_pair("Bob", "alice"), dm_pair("ALICE", "bob"));
        assert_eq!(dm_pair("Bob", "alice"), ("alice".into(), "bob".into()));
    }

    #[tokio::test]
    async fn the_first_acceptance_creates_the_crew_room_and_later_ones_invite() {
        let (rooms, store, matrix) = rooms();
        let post = Uuid::new_v4();
        let room = rooms.crew_member_added(post, "Host", "Bob").await.unwrap();
        let again = rooms
            .crew_member_added(post, "Host", "Carol")
            .await
            .unwrap();
        assert_eq!(room, again);
        assert_eq!(
            matrix.calls(),
            vec![
                format!("create {room} invite=@host:starstats.app,@bob:starstats.app mod=@host:starstats.app direct=false"),
                format!("invite {room} @carol:starstats.app"),
            ]
        );
        assert_eq!(store.members_of(&room), vec!["bob", "carol", "host"]);

        rooms.crew_member_removed(post, "Carol").await.unwrap();
        assert_eq!(
            matrix.calls().last().unwrap(),
            &format!("kick {room} @carol:starstats.app")
        );
        assert_eq!(store.members_of(&room), vec!["bob", "host"]);
    }

    #[tokio::test]
    async fn a_dm_is_made_once_and_ending_it_closes_it() {
        let (rooms, store, matrix) = rooms();
        let room = rooms.dm("Alice", "Bob").await.unwrap();
        assert_eq!(
            rooms.dm("bob", "ALICE").await.unwrap(),
            room,
            "one room per pair"
        );
        assert_eq!(matrix.calls().len(), 1);
        rooms.end_dm("Bob", "Alice").await.unwrap();
        assert!(store.members_of(&room).is_empty());
        assert_ne!(
            rooms.dm("Alice", "Bob").await.unwrap(),
            room,
            "a new one after"
        );
    }

    #[tokio::test]
    async fn closing_an_account_empties_its_rooms_then_erases_it() {
        let (rooms, store, matrix) = rooms();
        let dm = rooms.dm("Gone", "Alice").await.unwrap();
        rooms.close_account("GONE").await.unwrap();
        let calls = matrix.calls();
        assert_eq!(calls.last().unwrap(), "deactivate @gone:starstats.app");
        assert!(calls
            .iter()
            .any(|c| c == &format!("kick {dm} @gone:starstats.app")));
        assert_eq!(store.members_of(&dm), vec!["alice"]);
    }

    #[tokio::test]
    async fn remove_everywhere_takes_a_player_out_of_every_room() {
        let (rooms, store, matrix) = rooms();
        let crew = rooms
            .crew_member_added(Uuid::new_v4(), "Host", "Mallory")
            .await
            .unwrap();
        let dm = rooms.dm("Mallory", "Alice").await.unwrap();
        let other = rooms.dm("Alice", "Bob").await.unwrap();
        rooms
            .remove_everywhere("MALLORY", "Restricted from chat")
            .await
            .unwrap();
        let kicks: Vec<String> = matrix
            .calls()
            .into_iter()
            .filter(|c| c.starts_with("kick"))
            .collect();
        assert_eq!(kicks.len(), 2);
        assert!(kicks.iter().all(|k| k.ends_with("@mallory:starstats.app")));
        assert_eq!(store.members_of(&crew), vec!["host"]);
        assert_eq!(store.members_of(&dm), vec!["alice"]);
        assert_eq!(
            store.members_of(&other),
            vec!["alice", "bob"],
            "nobody else moved"
        );
    }

    /// The store's SQL against the real schema. Skipped without
    /// STARSTATS_TEST_DATABASE_URL.
    #[tokio::test]
    async fn postgres_chat_room_store_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres chat room test");
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
        let s = PostgresChatRoomStore::new(pool);
        let tag = &Uuid::new_v4().simple().to_string()[..8];
        let (crew, dm) = (format!("!crew{tag}:t"), format!("!dm{tag}:t"));
        let (alice, bob) = (format!("Alice{tag}"), format!("Bob{tag}"));
        let post = Uuid::new_v4();

        s.insert_room(&crew, RoomKind::Crew, Some(post), None)
            .await
            .unwrap();
        s.insert_room(&dm, RoomKind::Dm, None, Some(dm_pair(&bob, &alice)))
            .await
            .unwrap();
        for (room, h) in [(&crew, &alice), (&crew, &bob), (&dm, &alice), (&dm, &bob)] {
            s.add_member(room, h).await.unwrap();
        }
        s.add_member(&crew, &alice.to_uppercase()).await.unwrap(); // idempotent
        assert_eq!(
            s.crew_room(post).await.unwrap().as_deref(),
            Some(crew.as_str())
        );
        assert_eq!(
            s.dm_room(&alice.to_uppercase(), &bob)
                .await
                .unwrap()
                .as_deref(),
            Some(dm.as_str()),
            "either order, any case"
        );
        let mine = s.rooms_of(&alice).await.unwrap();
        assert_eq!(mine.len(), 2);
        let d = mine.iter().find(|r| r.kind == RoomKind::Dm).unwrap();
        assert_eq!(d.other_handle.as_deref(), Some(bob.to_lowercase().as_str()));

        assert!(s.is_member(&crew, &alice.to_uppercase()).await.unwrap());
        s.remove_member(&crew, &alice).await.unwrap();
        assert!(!s.is_member(&crew, &alice).await.unwrap());
        assert_eq!(s.rooms_of(&alice).await.unwrap().len(), 1);
        s.close_room(&dm).await.unwrap();
        assert!(s.dm_room(&alice, &bob).await.unwrap().is_none());
        assert!(s.rooms_of(&alice).await.unwrap().is_empty());
        // A closed DM does not stop a new one for the same pair.
        let dm2 = format!("!dm2{tag}:t");
        s.insert_room(&dm2, RoomKind::Dm, None, Some(dm_pair(&alice, &bob)))
            .await
            .unwrap();
        assert_eq!(
            s.dm_room(&alice, &bob).await.unwrap().as_deref(),
            Some(dm2.as_str())
        );
    }

    /// The HTTP client against a real Synapse built from infra/synapse
    /// (the chat guard included). Skipped unless STARSTATS_TEST_SYNAPSE_URL
    /// and STARSTATS_TEST_SYNAPSE_AS_TOKEN are set.
    #[tokio::test]
    async fn live_synapse_rooms_are_what_we_say() {
        let (Ok(url), Ok(token)) = (
            std::env::var("STARSTATS_TEST_SYNAPSE_URL"),
            std::env::var("STARSTATS_TEST_SYNAPSE_AS_TOKEN"),
        ) else {
            eprintln!("STARSTATS_TEST_SYNAPSE_URL unset — skipping live Synapse test");
            return;
        };
        let service = "@starstats:starstats.app".to_string();
        let m = HttpMatrixRooms::new(&url, token.clone(), service.clone()).unwrap();
        let tag = &Uuid::new_v4().simple().to_string()[..8];
        let (host, bob, carol) = (
            format!("@host{tag}:starstats.app"),
            format!("@bob{tag}:starstats.app"),
            format!("@carol{tag}:starstats.app"),
        );
        let room = m
            .create_room(&[host.clone(), bob.clone()], Some(&host), false)
            .await
            .unwrap();
        m.invite(&room, &carol).await.unwrap();
        m.kick(&room, &carol, "Removed from the crew")
            .await
            .unwrap();

        let http = reqwest::Client::new();
        let state: Vec<serde_json::Value> = http
            .get(format!(
                "{url}/_matrix/client/v3/rooms/{}/state?user_id={}",
                enc(&room),
                enc(&service)
            ))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let find = |t: &str, k: &str| {
            state
                .iter()
                .find(|e| e["type"] == t && e["state_key"] == k)
                .cloned()
                .unwrap_or_default()
        };
        assert_eq!(find("m.room.create", "")["content"]["m.federate"], false);
        assert_eq!(
            find("m.room.encryption", "")["content"]["algorithm"],
            "m.megolm.v1.aes-sha2"
        );
        assert_eq!(find("m.room.name", ""), serde_json::Value::Null, "no name");
        let pl = find("m.room.power_levels", "")["content"].clone();
        assert_eq!(pl["users"][&service], 100);
        assert_eq!(pl["users"][&host], 50);
        assert_eq!(
            find("m.room.member", &bob)["content"]["membership"],
            "invite"
        );
        assert_eq!(
            find("m.room.member", &carol)["content"]["membership"],
            "leave"
        );
    }
}
