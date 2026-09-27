//! Friends-only presence and the realtime gateway's fan-out (social phase 3).
//!
//! ## What is shared, and with whom
//!
//! Presence is coarse: offline, online, in game, in quantum. The star
//! system is a second, separate opt-in. Nothing finer than the system is
//! accepted, and only friends ever see any of it.
//!
//! Two gates, both of which must be open (the same model as cloud sync):
//! the tray's own "share presence" setting decides whether it reports at
//! all, and the server-side `users.presence_level` (`off` by default)
//! decides whether a report is kept and passed on. Either side can
//! withdraw.
//!
//! ## What is kept
//!
//! Nothing beyond memory. [`PresenceHub`] holds each user's latest state
//! and forgets it after [`PRESENCE_TTL`] without a heartbeat, on
//! disconnect, when the level goes to `off`, and on restart. There is no
//! history and no table for it; the only stored value is the level.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;
use utoipa::ToSchema;

/// A report older than this reads as offline. The tray heartbeats every
/// minute, so three missed beats.
pub const PRESENCE_TTL: chrono::Duration = chrono::Duration::minutes(3);

/// An unchanged report within this window only refreshes the timestamp;
/// it is not fanned out again.
pub const REFANOUT_AFTER: chrono::Duration = chrono::Duration::seconds(30);

/// Pushes queued per connection before new ones are dropped. A tray that
/// stops reading loses pushes, not the server's memory.
pub const CONNECTION_QUEUE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Online,
    InGame,
    InQuantum,
}

/// How much of a user's presence their friends may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresenceLevel {
    /// Nothing is kept or shown. The default.
    Off,
    /// Offline / online / in game / in quantum.
    Status,
    /// Status, plus the star system.
    System,
}

impl PresenceLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Status => "status",
            Self::System => "system",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "off" => Self::Off,
            "status" => Self::Status,
            "system" => Self::System,
            _ => return None,
        })
    }
}

/// What a tray reports.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PresenceUpdate {
    pub state: PresenceState,
    /// Star system name, e.g. "Stanton". Kept only when the level is
    /// `system`, and only if it looks like a system name.
    #[serde(default)]
    pub system: Option<String>,
}

/// One friend's presence as another friend sees it. `state: null` is
/// offline, which is also what "not sharing" looks like: a friend cannot
/// tell the two apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FriendPresence {
    pub handle: String,
    pub state: Option<PresenceState>,
    pub system: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl FriendPresence {
    pub fn offline(handle: &str) -> Self {
        Self {
            handle: handle.to_string(),
            state: None,
            system: None,
            updated_at: None,
        }
    }
}

/// Messages pushed to a connected tray.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// A friend's presence changed.
    Presence(FriendPresence),
    /// A notification arrived; fetch the inbox now rather than at the
    /// next poll.
    Notification,
}

/// Messages a tray sends.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Presence(PresenceUpdate),
    /// Stop showing me as present (the game closed, or sharing was
    /// turned off in the tray).
    Offline,
    Ping,
}

/// A system name is at most a few words of letters, digits, spaces,
/// hyphens and apostrophes. Anything else is dropped rather than shown
/// to friends: this field is free text from a client.
pub fn clean_system(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim();
    let ok = !s.is_empty()
        && s.chars().count() <= 32
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '\'');
    ok.then(|| s.to_string())
}

#[derive(Debug, Clone)]
struct Entry {
    handle: String,
    state: PresenceState,
    system: Option<String>,
    updated_at: DateTime<Utc>,
}

/// In-memory presence and the per-user push channels.
#[derive(Default)]
pub struct PresenceHub {
    entries: Mutex<HashMap<String, Entry>>,
    conns: Mutex<HashMap<String, Vec<(u64, mpsc::Sender<ServerMessage>)>>>,
    next_id: AtomicU64,
}

impl PresenceHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a report. Returns `true` when friends should be told: the
    /// state or system changed, or the last fan-out is stale.
    pub fn set(
        &self,
        handle: &str,
        state: PresenceState,
        system: Option<String>,
        now: DateTime<Utc>,
    ) -> bool {
        let mut entries = self.entries.lock().unwrap();
        let key = handle.to_lowercase();
        let changed = match entries.get(&key) {
            Some(e) => {
                e.state != state || e.system != system || now - e.updated_at >= REFANOUT_AFTER
            }
            None => true,
        };
        entries.insert(
            key,
            Entry {
                handle: handle.to_string(),
                state,
                system,
                updated_at: now,
            },
        );
        changed
    }

    /// Forget a user's presence. `true` if there was any to forget.
    pub fn clear(&self, handle: &str) -> bool {
        self.entries
            .lock()
            .unwrap()
            .remove(&handle.to_lowercase())
            .is_some()
    }

    /// `handle`'s presence as a friend sees it at `level`, or offline
    /// when unknown, stale, or not shared.
    pub fn view(&self, handle: &str, level: PresenceLevel, now: DateTime<Utc>) -> FriendPresence {
        if level == PresenceLevel::Off {
            return FriendPresence::offline(handle);
        }
        let entries = self.entries.lock().unwrap();
        match entries.get(&handle.to_lowercase()) {
            Some(e) if now - e.updated_at < PRESENCE_TTL => FriendPresence {
                handle: e.handle.clone(),
                state: Some(e.state),
                system: if level == PresenceLevel::System {
                    e.system.clone()
                } else {
                    None
                },
                updated_at: Some(e.updated_at),
            },
            _ => FriendPresence::offline(handle),
        }
    }

    /// Drop entries past the TTL, so memory holds nobody who left.
    /// Returns the handles dropped, for an offline fan-out.
    pub fn sweep(&self, now: DateTime<Utc>) -> Vec<String> {
        let mut entries = self.entries.lock().unwrap();
        let stale: Vec<String> = entries
            .iter()
            .filter(|(_, e)| now - e.updated_at >= PRESENCE_TTL)
            .map(|(k, _)| k.clone())
            .collect();
        stale
            .into_iter()
            .filter_map(|k| entries.remove(&k).map(|e| e.handle))
            .collect()
    }

    /// Open a push channel for one connection of `handle`.
    pub fn register(&self, handle: &str) -> (u64, mpsc::Receiver<ServerMessage>) {
        let (tx, rx) = mpsc::channel(CONNECTION_QUEUE);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.conns
            .lock()
            .unwrap()
            .entry(handle.to_lowercase())
            .or_default()
            .push((id, tx));
        (id, rx)
    }

    /// Close one connection. `true` when it was the user's last.
    pub fn unregister(&self, handle: &str, id: u64) -> bool {
        let mut conns = self.conns.lock().unwrap();
        let key = handle.to_lowercase();
        let Some(list) = conns.get_mut(&key) else {
            return true;
        };
        list.retain(|(i, _)| *i != id);
        if list.is_empty() {
            conns.remove(&key);
            true
        } else {
            false
        }
    }

    /// Push to every connection `handle` has open. Never blocks: a full
    /// queue drops the message, a closed one is pruned.
    pub fn send_to(&self, handle: &str, msg: ServerMessage) {
        let mut conns = self.conns.lock().unwrap();
        if let Some(list) = conns.get_mut(&handle.to_lowercase()) {
            list.retain(|(_, tx)| {
                !matches!(
                    tx.try_send(msg.clone()),
                    Err(mpsc::error::TrySendError::Closed(_))
                )
            });
        }
    }
}

// -- The server-side gate --------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum PresenceError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("stored value out of domain: {0}")]
    Domain(String),
}

#[async_trait]
pub trait PresenceSettingsStore: Send + Sync + 'static {
    /// `Off` for a user who never chose, or who does not exist.
    async fn level(&self, handle: &str) -> Result<PresenceLevel, PresenceError>;
    async fn set_level(&self, handle: &str, level: PresenceLevel) -> Result<(), PresenceError>;
    /// Levels for many users at once, keyed by lower-cased handle. A
    /// user absent from the map is `Off`.
    async fn levels(
        &self,
        handles: &[String],
    ) -> Result<HashMap<String, PresenceLevel>, PresenceError>;
}

pub struct PostgresPresenceSettingsStore {
    pool: PgPool,
}

impl PostgresPresenceSettingsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PresenceSettingsStore for PostgresPresenceSettingsStore {
    async fn level(&self, handle: &str) -> Result<PresenceLevel, PresenceError> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT presence_level FROM users WHERE lower(claimed_handle) = lower($1)",
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        match row.and_then(|(l,)| l) {
            None => Ok(PresenceLevel::Off),
            Some(l) => PresenceLevel::parse(&l)
                .ok_or_else(|| PresenceError::Domain(format!("presence_level={l}"))),
        }
    }

    async fn set_level(&self, handle: &str, level: PresenceLevel) -> Result<(), PresenceError> {
        sqlx::query("UPDATE users SET presence_level = $2 WHERE lower(claimed_handle) = lower($1)")
            .bind(handle)
            .bind(level.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn levels(
        &self,
        handles: &[String],
    ) -> Result<HashMap<String, PresenceLevel>, PresenceError> {
        if handles.is_empty() {
            return Ok(HashMap::new());
        }
        let lowered: Vec<String> = handles.iter().map(|h| h.to_lowercase()).collect();
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT lower(claimed_handle), presence_level FROM users
             WHERE lower(claimed_handle) = ANY($1) AND presence_level IS NOT NULL",
        )
        .bind(&lowered)
        .fetch_all(&self.pool)
        .await?;
        // An unknown value reads as Off: never share on a value we do
        // not understand.
        Ok(rows
            .into_iter()
            .filter_map(|(h, l)| PresenceLevel::parse(&l).map(|l| (h, l)))
            .collect())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    #[derive(Default)]
    pub struct MemoryPresenceSettingsStore {
        levels: Mutex<HashMap<String, PresenceLevel>>,
    }

    impl MemoryPresenceSettingsStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl PresenceSettingsStore for MemoryPresenceSettingsStore {
        async fn level(&self, handle: &str) -> Result<PresenceLevel, PresenceError> {
            Ok(*self
                .levels
                .lock()
                .unwrap()
                .get(&handle.to_lowercase())
                .unwrap_or(&PresenceLevel::Off))
        }

        async fn set_level(&self, handle: &str, level: PresenceLevel) -> Result<(), PresenceError> {
            self.levels
                .lock()
                .unwrap()
                .insert(handle.to_lowercase(), level);
            Ok(())
        }

        async fn levels(
            &self,
            handles: &[String],
        ) -> Result<HashMap<String, PresenceLevel>, PresenceError> {
            let levels = self.levels.lock().unwrap();
            Ok(handles
                .iter()
                .filter_map(|h| {
                    let k = h.to_lowercase();
                    levels.get(&k).map(|l| (k, *l))
                })
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + s, 0).unwrap()
    }

    #[test]
    fn system_names_are_checked_not_trusted() {
        assert_eq!(clean_system(Some(" Stanton ")), Some("Stanton".into()));
        assert_eq!(clean_system(Some("Nyx")), Some("Nyx".into()));
        assert_eq!(clean_system(Some("<script>")), None);
        assert_eq!(clean_system(Some("Stanton / ARC-L1 at 1.2.3")), None);
        assert_eq!(clean_system(Some(&"x".repeat(33))), None);
        assert_eq!(clean_system(Some("")), None);
        assert_eq!(clean_system(None), None);
    }

    #[test]
    fn friends_see_only_what_the_level_allows() {
        let hub = PresenceHub::new();
        hub.set("Alice", PresenceState::InGame, Some("Stanton".into()), t(0));
        let off = hub.view("alice", PresenceLevel::Off, t(1));
        assert_eq!(off.state, None, "off looks exactly like offline");
        let status = hub.view("alice", PresenceLevel::Status, t(1));
        assert_eq!(status.state, Some(PresenceState::InGame));
        assert_eq!(status.system, None, "the system is its own opt-in");
        let system = hub.view("ALICE", PresenceLevel::System, t(1));
        assert_eq!(system.system.as_deref(), Some("Stanton"));
        assert_eq!(system.handle, "Alice");
    }

    #[test]
    fn a_report_goes_stale_and_is_swept() {
        let hub = PresenceHub::new();
        hub.set("Alice", PresenceState::Online, None, t(0));
        let later = t(0) + PRESENCE_TTL;
        assert_eq!(hub.view("Alice", PresenceLevel::Status, later).state, None);
        assert_eq!(hub.sweep(later), vec!["Alice".to_string()]);
        assert!(!hub.clear("Alice"), "nothing left in memory");
    }

    #[test]
    fn only_a_change_or_a_stale_fanout_is_passed_on() {
        let hub = PresenceHub::new();
        assert!(hub.set("Alice", PresenceState::Online, None, t(0)));
        assert!(
            !hub.set("Alice", PresenceState::Online, None, t(5)),
            "heartbeat"
        );
        assert!(
            hub.set("Alice", PresenceState::InGame, None, t(6)),
            "state changed"
        );
        assert!(hub.set("Alice", PresenceState::InGame, None, t(6) + REFANOUT_AFTER));
    }

    #[tokio::test]
    async fn pushes_reach_every_connection_and_stop_at_the_last_close() {
        let hub = PresenceHub::new();
        let (a, mut rx_a) = hub.register("Bob");
        let (b, mut rx_b) = hub.register("bob");
        hub.send_to("BOB", ServerMessage::Notification);
        assert_eq!(rx_a.recv().await, Some(ServerMessage::Notification));
        assert_eq!(rx_b.recv().await, Some(ServerMessage::Notification));
        assert!(!hub.unregister("Bob", a), "one connection is still open");
        assert!(hub.unregister("Bob", b));
    }

    #[test]
    fn a_slow_reader_loses_pushes_not_the_server() {
        let hub = PresenceHub::new();
        let (_id, _rx) = hub.register("Bob");
        for _ in 0..CONNECTION_QUEUE * 2 {
            hub.send_to("Bob", ServerMessage::Notification);
        }
    }

    #[test]
    fn messages_have_the_documented_wire_shape() {
        let msg = ServerMessage::Presence(FriendPresence {
            handle: "Alice".into(),
            state: Some(PresenceState::InQuantum),
            system: None,
            updated_at: None,
        });
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], "presence");
        assert_eq!(v["state"], "in_quantum");
        let c: ClientMessage =
            serde_json::from_str(r#"{"type":"presence","state":"in_game","system":"Pyro"}"#)
                .unwrap();
        assert!(matches!(c, ClientMessage::Presence(u) if u.system.as_deref() == Some("Pyro")));
        assert!(matches!(
            serde_json::from_str::<ClientMessage>(r#"{"type":"offline"}"#).unwrap(),
            ClientMessage::Offline
        ));
    }
}
