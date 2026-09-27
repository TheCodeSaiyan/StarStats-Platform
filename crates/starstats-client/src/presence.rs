//! Friends-only presence from the tray (social phase 3).
//!
//! Holds one WebSocket to the StarStats API's realtime gateway
//! (`/v1/ws`) while the tray is paired. Over it the tray:
//!
//! - **receives** friends' presence (kept for the Social pane) and a
//!   nudge when a notification arrives, which wakes the notification
//!   poller at once;
//! - **reports** its own presence, but only while `social.share_presence`
//!   is on. That is the tray's half of a two-gate model: the server keeps
//!   and passes on a report only if the account's presence setting is
//!   also on, and the star system only if that setting says so.
//!
//! What is reported is coarse: online (tray running, game not), in game,
//! or in quantum, and the star system. It is worked out from events the
//! Game.log tail already stored, the way the org connector does it; this
//! reads nothing new. The system comes from the shared location
//! classifier, so it is always a catalogue system name, never a place
//! within one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use starstats_core::events::{GameEvent, QuantumTargetPhase};
use starstats_core::location_catalog::LocationCatalog;
use starstats_core::location_classifier::classify;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use crate::storage::Storage;

/// How often the tray works out its state.
const TICK: Duration = Duration::from_secs(20);
/// An unchanged report is resent this often, as the heartbeat that keeps
/// it from expiring server-side (three minutes).
const HEARTBEAT: Duration = Duration::from_secs(60);
/// Recent events examined to work out the state. Enough to find the last
/// location in any session; cheap to parse every tick.
const LOOKBACK: usize = 200;
const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(120);

/// What the tray reports: the server's `PresenceUpdate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresenceUpdate {
    pub state: &'static str,
    pub system: Option<String>,
}

/// One friend's presence as the gateway pushes it. `state: None` is
/// offline, or not sharing; the two look the same on purpose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendPresence {
    pub handle: String,
    pub state: Option<String>,
    pub system: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Presence(FriendPresence),
    Notification,
    #[serde(other)]
    Unknown,
}

// -- Friends' presence, for the Social pane --------------------------------

fn friends() -> &'static Mutex<HashMap<String, FriendPresence>> {
    static FRIENDS: OnceLock<Mutex<HashMap<String, FriendPresence>>> = OnceLock::new();
    FRIENDS.get_or_init(Default::default)
}

/// Friends' presence as last pushed. Empty while disconnected.
pub fn friends_snapshot() -> Vec<FriendPresence> {
    let mut v: Vec<FriendPresence> = friends().lock().unwrap().values().cloned().collect();
    v.sort_by_key(|p| p.handle.to_lowercase());
    v
}

fn clear_friends() {
    friends().lock().unwrap().clear();
}

// -- Working out the tray's own state ----------------------------------------

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub system: Option<String>,
    pub in_quantum: bool,
}

/// The system and quantum state from events, newest first. The newest
/// quantum signal wins (a jump started, or a location reached since); the
/// system is the newest one a location resolves to. A quantum
/// destination is not where the player is, so it never sets the system.
pub fn facts(newest_first: &[GameEvent], catalog: &LocationCatalog) -> Facts {
    let mut in_quantum: Option<bool> = None;
    let mut system: Option<String> = None;
    for ev in newest_first {
        if in_quantum.is_none() {
            in_quantum = match ev {
                GameEvent::QuantumTargetSelected(e) if e.phase == QuantumTargetPhase::Selected => {
                    Some(true)
                }
                GameEvent::QuantumRoute(_) => Some(true),
                GameEvent::QuantumArrived(_) => Some(false),
                GameEvent::LocationChanged(_)
                | GameEvent::LocationInventoryRequested(_)
                | GameEvent::PlanetTerrainLoad(_)
                | GameEvent::VehicleStowed(_)
                | GameEvent::SeedSolarSystem(_) => Some(false),
                _ => None,
            };
        }
        if system.is_none() {
            let raw = match ev {
                GameEvent::SeedSolarSystem(e) if e.success => Some(e.solar_system.as_str()),
                GameEvent::QuantumRoute(e) => Some(e.start_system.as_str()),
                GameEvent::QuantumTargetSelected(_) => None,
                other => other.location_raw(),
            };
            system = raw.and_then(|r| classify(r, catalog).system);
        }
        if in_quantum.is_some() && system.is_some() {
            break;
        }
    }
    Facts {
        system,
        in_quantum: in_quantum.unwrap_or(false),
    }
}

/// Where the player is and what they are flying, for pre-filling a
/// Looking for Group post. Worked out from the same stored events as
/// presence; every field is a suggestion the player can change.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct WhereAmI {
    pub system: Option<String>,
    /// The friendly name of the newest place the player was, e.g. a
    /// station or city.
    pub location: Option<String>,
    /// The ship of the newest quantum jump, e.g. "RSI Constellation Phoenix".
    pub ship: Option<String>,
}

/// A readable ship name from a vehicle class: underscores become spaces
/// and a trailing numeric instance id is dropped
/// (`AEGS_Avenger_Titan_1234` → `AEGS Avenger Titan`).
pub fn ship_name(vehicle_class: &str) -> Option<String> {
    let mut parts: Vec<&str> = vehicle_class.split('_').filter(|p| !p.is_empty()).collect();
    if parts.len() > 1
        && parts
            .last()
            .is_some_and(|p| p.chars().all(|c| c.is_ascii_digit()))
    {
        parts.pop();
    }
    let name = parts.join(" ");
    (!name.is_empty()).then_some(name)
}

pub fn where_am_i(newest_first: &[GameEvent], catalog: &LocationCatalog) -> WhereAmI {
    let mut location: Option<String> = None;
    let mut ship: Option<String> = None;
    for ev in newest_first {
        if location.is_none() {
            let raw = match ev {
                // A quantum destination is where the player is going.
                GameEvent::QuantumTargetSelected(_) => None,
                other => other.location_raw(),
            };
            location = raw.map(|r| classify(r, catalog).display_name);
        }
        if ship.is_none() {
            ship = match ev {
                GameEvent::QuantumTargetSelected(e) => ship_name(&e.vehicle_class),
                GameEvent::QuantumRoute(e) => ship_name(&e.vehicle_class),
                GameEvent::QuantumArrived(e) => ship_name(&e.vehicle_class),
                _ => None,
            };
        }
        if location.is_some() && ship.is_some() {
            break;
        }
    }
    WhereAmI {
        system: facts(newest_first, catalog).system,
        location,
        ship,
    }
}

/// [`where_am_i`] from the tray's stored events.
pub fn current_where(storage: &Storage, catalog: &RwLock<Arc<LocationCatalog>>) -> WhereAmI {
    let events: Vec<GameEvent> = storage
        .recent_events(LOOKBACK)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| serde_json::from_str::<GameEvent>(&r.payload_json).ok())
        .collect();
    let cat = catalog.read().clone();
    where_am_i(&events, &cat)
}

/// The report for a moment: with the game closed the tray is merely
/// online, and says nothing about where the player last was.
pub fn report_for(game_running: bool, facts: Facts) -> PresenceUpdate {
    if !game_running {
        return PresenceUpdate {
            state: "online",
            system: None,
        };
    }
    PresenceUpdate {
        state: if facts.in_quantum {
            "in_quantum"
        } else {
            "in_game"
        },
        system: facts.system,
    }
}

fn current_report(storage: &Storage, catalog: &RwLock<Arc<LocationCatalog>>) -> PresenceUpdate {
    let running = crate::process_guard::is_starcitizen_running();
    let events: Vec<GameEvent> = if running {
        storage
            .recent_events(LOOKBACK)
            .unwrap_or_default()
            .iter()
            .filter_map(|r| serde_json::from_str::<GameEvent>(&r.payload_json).ok())
            .collect()
    } else {
        Vec::new()
    };
    let cat = catalog.read().clone();
    report_for(running, facts(&events, &cat))
}

// -- The connection ------------------------------------------------------------

fn request(url: &str, token: &str) -> Option<tokio_tungstenite::tungstenite::http::Request<()>> {
    let mut req = url.into_client_request().ok()?;
    let value = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
    req.headers_mut().insert(AUTHORIZATION, value);
    Some(req)
}

/// Run for the life of the app. Re-reads config on every connect, so
/// pairing, unpairing and the share setting apply without a restart.
pub async fn run(
    app: tauri::AppHandle,
    storage: Arc<Storage>,
    catalog: Arc<RwLock<Arc<LocationCatalog>>>,
) {
    let mut backoff = BACKOFF_MIN;
    loop {
        let client = crate::config::load()
            .ok()
            .and_then(|cfg| crate::social::SocialClient::from_config(&cfg).ok());
        let Some(client) = client else {
            // Unpaired: nobody to be present to.
            clear_friends();
            tokio::time::sleep(Duration::from_secs(30)).await;
            continue;
        };
        let (url, token) = client.gateway();
        let Some(req) = request(&url, &token) else {
            tracing::warn!("presence: gateway request could not be built");
            tokio::time::sleep(BACKOFF_MAX).await;
            continue;
        };
        match tokio_tungstenite::connect_async(req).await {
            Ok((socket, _)) => {
                let started = tokio::time::Instant::now();
                if let Err(e) = session(socket, &app, &storage, &catalog).await {
                    tracing::debug!(error = %e, "presence: session ended");
                }
                // Reset only after a session that held, so a server that
                // accepts and drops at once cannot make this a hot loop.
                backoff = if started.elapsed() > Duration::from_secs(60) {
                    BACKOFF_MIN
                } else {
                    (backoff * 2).min(BACKOFF_MAX)
                };
            }
            Err(e) => {
                tracing::debug!(error = %e, "presence: connect failed");
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
        clear_friends();
        tokio::time::sleep(backoff).await;
    }
}

async fn session<S>(
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    app: &tauri::AppHandle,
    storage: &Storage,
    catalog: &RwLock<Arc<LocationCatalog>>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tauri::Emitter;
    let mut tick = tokio::time::interval(TICK);
    let mut last: Option<(PresenceUpdate, tokio::time::Instant)> = None;
    let mut last_send = tokio::time::Instant::now();
    loop {
        tokio::select! {
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(ServerMessage::Presence(p)) => {
                        friends()
                            .lock()
                            .unwrap()
                            .insert(p.handle.to_lowercase(), p.clone());
                        let _ = app.emit("friend-presence", p);
                    }
                    Ok(ServerMessage::Notification) => crate::social::nudge(),
                    Ok(ServerMessage::Unknown) | Err(_) => {}
                },
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(_)) => {}
            },
            _ = tick.tick() => {
                let share = crate::config::load()
                    .map(|c| c.social.share_presence)
                    .unwrap_or(false);
                let now = tokio::time::Instant::now();
                let frame = if share {
                    let report = current_report(storage, catalog);
                    let due = match &last {
                        Some((prev, at)) => *prev != report || now - *at >= HEARTBEAT,
                        None => true,
                    };
                    if due {
                        last = Some((report.clone(), now));
                        Some(serde_json::json!({
                            "type": "presence",
                            "state": report.state,
                            "system": report.system,
                        }))
                    } else {
                        None
                    }
                } else if last.take().is_some() {
                    // Sharing was just turned off: say so now rather than
                    // leaving friends to wait for the report to expire.
                    Some(serde_json::json!({ "type": "offline" }))
                } else if now - last_send >= HEARTBEAT {
                    // Nothing to report; keep the connection warm through
                    // proxies that close idle sockets.
                    Some(serde_json::json!({ "type": "ping" }))
                } else {
                    None
                };
                if let Some(frame) = frame {
                    socket.send(Message::Text(frame.to_string())).await?;
                    last_send = now;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starstats_core::events::{LocationChanged, QuantumArrived, QuantumRoute, SeedSolarSystem};

    fn catalog() -> LocationCatalog {
        LocationCatalog::default()
    }

    fn seed(system: &str) -> GameEvent {
        GameEvent::SeedSolarSystem(SeedSolarSystem {
            timestamp: "t".into(),
            solar_system: system.into(),
            shard: "s".into(),
            success: true,
        })
    }

    fn route(from: &str) -> GameEvent {
        GameEvent::QuantumRoute(QuantumRoute {
            timestamp: "t".into(),
            start_system: from.into(),
            destination: "OM-1".into(),
            vehicle_class: "v".into(),
            vehicle_id: "1".into(),
        })
    }

    fn arrived() -> GameEvent {
        GameEvent::QuantumArrived(QuantumArrived {
            timestamp: "t".into(),
            vehicle_class: "v".into(),
            vehicle_id: "1".into(),
        })
    }

    #[test]
    fn ship_names_are_readable() {
        assert_eq!(
            ship_name("AEGS_Avenger_Titan_1234").as_deref(),
            Some("AEGS Avenger Titan")
        );
        assert_eq!(
            ship_name("RSI_Constellation_Phoenix").as_deref(),
            Some("RSI Constellation Phoenix")
        );
        assert_eq!(
            ship_name("1234"),
            Some("1234".to_string()),
            "a lone number is kept"
        );
        assert_eq!(ship_name(""), None);
    }

    #[test]
    fn where_am_i_takes_the_newest_ship_and_ignores_destinations() {
        let cat = catalog();
        let w = where_am_i(&[route("Stanton"), arrived()], &cat);
        assert_eq!(w.ship.as_deref(), Some("v"));
        assert_eq!(w.location, None, "a quantum route is not a place");
    }

    #[test]
    fn with_the_game_closed_the_tray_is_only_online() {
        let r = report_for(
            false,
            Facts {
                system: Some("Stanton".into()),
                in_quantum: true,
            },
        );
        assert_eq!(r.state, "online");
        assert_eq!(r.system, None, "nothing about where the player last was");
    }

    #[test]
    fn the_newest_quantum_signal_wins() {
        let cat = catalog();
        assert!(facts(&[route("Stanton"), arrived()], &cat).in_quantum);
        assert!(!facts(&[arrived(), route("Stanton")], &cat).in_quantum);
        assert!(!facts(&[], &cat).in_quantum, "no signal is not in quantum");
    }

    #[test]
    fn a_report_is_in_game_or_in_quantum_while_playing() {
        let r = report_for(
            true,
            Facts {
                system: None,
                in_quantum: false,
            },
        );
        assert_eq!(r.state, "in_game");
        let r = report_for(
            true,
            Facts {
                system: None,
                in_quantum: true,
            },
        );
        assert_eq!(r.state, "in_quantum");
    }

    #[test]
    fn the_system_comes_only_from_the_classifier() {
        // With an empty catalogue nothing resolves to a system, so the
        // raw log text is never passed on as one.
        let cat = catalog();
        let f = facts(
            &[
                GameEvent::LocationChanged(LocationChanged {
                    timestamp: "t".into(),
                    from: None,
                    to: "RR_HUR_LEO_int".into(),
                }),
                seed("Stanton"),
            ],
            &cat,
        );
        assert_eq!(f.system, classify("Stanton", &cat).system);
    }

    #[test]
    fn server_messages_decode_and_unknown_kinds_are_ignored() {
        let m: ServerMessage = serde_json::from_str(
            r#"{"type":"presence","handle":"Alice","state":"in_game","system":"Pyro","updated_at":null}"#,
        )
        .unwrap();
        assert!(matches!(m, ServerMessage::Presence(p) if p.system.as_deref() == Some("Pyro")));
        assert!(matches!(
            serde_json::from_str::<ServerMessage>(r#"{"type":"notification"}"#).unwrap(),
            ServerMessage::Notification
        ));
        assert!(matches!(
            serde_json::from_str::<ServerMessage>(r#"{"type":"lfg_invite","x":1}"#).unwrap(),
            ServerMessage::Unknown
        ));
    }

    #[test]
    fn the_gateway_request_carries_the_token_in_a_header() {
        let req = request("wss://api.example/v1/ws", "tok").unwrap();
        assert_eq!(req.headers()[AUTHORIZATION], "Bearer tok");
        assert!(!req.uri().to_string().contains("tok"), "never in the URL");
    }
}
