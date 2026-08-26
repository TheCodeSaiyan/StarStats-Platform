//! Opt-in connector to a self-hosted **org platform** (the `orgplatform`
//! companion project). It rides the SAME read-only `Game.log` boundary
//! StarStats already crosses: it never reads anything new, it re-uses the
//! events the tail already parsed into local SQLite, projects each one
//! down to a tiny self-reported [`Telemetry`] (zone / quantum-state / a
//! discrete spawn-kill-death-downed marker — never coordinates), and
//! pushes it to the org platform's authenticated member channel
//! (`/ws/member`, authenticating with an `Authorization: Bearer` header).
//!
//! Wire contract: the org platform's `hud-protocol` crate. We do NOT
//! take a cross-repo path dependency on it (that couples two unrelated
//! release trains); instead we mirror the one message shape we emit —
//! `ClientMessage::Telemetry`, which serialises to
//! `{"type":"telemetry","zone":…,"quantum_state":…,"event":…}` — and
//! lock it down with a round-trip test. If the org platform bumps
//! `PROTOCOL_VERSION` in a way that changes that shape, the test here is
//! the canary.
//!
//! Disabled unless the user opts in via `[org_connector]` in
//! `config.toml` (`enabled = true` + a `platform_url` + a `bearer_token`).
//! Inbound `OrgContext` frames are drained and ignored — surfacing the
//! org-around-you picture in the tray UI is a separate, later feature;
//! this connector is the outbound half of Gap 4.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde::Serialize;
use starstats_core::events::{GameEvent, QuantumTargetPhase};
use starstats_core::location_catalog::LocationCatalog;
use starstats_core::location_classifier::classify;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use crate::config::OrgConnectorConfig;
use crate::storage::Storage;

/// Synthetic `tail_cursor` key under which we persist the id of the last
/// event we considered. Keyed in the same table the byte-offset tail
/// cursors live in, but the stored value is an event-row id, not a file
/// offset — the column is a bare integer, so the table doesn't care.
const CURSOR_KEY: &str = "__org_connector_event_cursor__";

/// How often we poll local SQLite for newly-parsed events to forward.
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Max events drained per poll. Telemetry is tiny; this only bounds the
/// catch-up burst after a reconnect.
const BATCH: usize = 256;

/// Min/max reconnect backoff. Plain exponential between connect attempts.
const BACKOFF_MIN: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Local mirror of the org platform's `hud-protocol::ClientMessage`,
/// reduced to the one variant this connector emits. The `#[serde(tag =
/// "type", rename_all = "snake_case")]` representation makes the wire
/// bytes byte-identical to the real enum's `Telemetry` arm.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OrgClientMessage {
    Telemetry(Telemetry),
}

/// Self-reported, read-only game state — mirror of
/// `hud-protocol::Telemetry`. Fields are skipped when `None` so frames
/// stay small; serde deserialises a missing `Option` field back to
/// `None` on the server, so the omission is wire-compatible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
struct Telemetry {
    #[serde(skip_serializing_if = "Option::is_none")]
    zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantum_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<String>,
}

/// Spawn the connector task if the user has opted in.
///
/// Returns `Some(handle)` when a task was spawned, `None` when the
/// connector is disabled or misconfigured (missing URL / token / bad
/// URL scheme). The caller may pass the handle to
/// [`respawn`] to abort and replace the worker on config change.
///
/// `tail_kick` is the shared [`Notify`] the Game.log tail fires after a
/// drain that ingested new events. The session loop waits on it so a
/// location change / death / spawn is forwarded to the org platform the
/// instant it lands locally, instead of waiting up to `POLL_INTERVAL`.
pub fn spawn(
    storage: Arc<Storage>,
    catalog: Arc<RwLock<Arc<LocationCatalog>>>,
    cfg: OrgConnectorConfig,
    tail_kick: Arc<Notify>,
) -> Option<tauri::async_runtime::JoinHandle<()>> {
    if !cfg.enabled {
        tracing::debug!("org connector disabled — not spawning");
        return None;
    }
    let (Some(platform_url), Some(bearer_token)) = (cfg.platform_url, cfg.bearer_token) else {
        tracing::warn!("org connector enabled but platform_url / bearer_token missing — skipping");
        return None;
    };
    let Some(ws_url) = build_ws_url(&platform_url) else {
        tracing::warn!(
            platform_url,
            "org connector: platform_url did not parse (or is plaintext to a \
             non-loopback host) — skipping"
        );
        return None;
    };

    let handle =
        tauri::async_runtime::spawn(run_loop(ws_url, bearer_token, storage, catalog, tail_kick));
    tracing::info!("org connector spawned");
    Some(handle)
}

/// Abort the currently-running org-connector worker (if any) and spawn a
/// fresh one with the current persisted config. Used by `save_config` to
/// pick up a changed `platform_url`, `bearer_token`, or `enabled` flag
/// without requiring an app restart.
///
/// Idempotent: when the new config also disables the connector (or is
/// incomplete) this leaves `org_connector_handle` as `None`, which is
/// the same state the boot path produces.
///
/// Reads config from disk so the caller doesn't have to thread the fresh
/// config in — there is exactly one place that mutates it
/// (`config::save`), and it is always called before this helper.
pub fn respawn(
    storage: Arc<Storage>,
    catalog: Arc<RwLock<Arc<LocationCatalog>>>,
    handle: Arc<parking_lot::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    tail_kick: Arc<Notify>,
) {
    // Abort first so the old worker stops connecting with stale credentials
    // before we spawn a fresh one. `abort()` is non-blocking.
    {
        let mut guard = handle.lock();
        if let Some(h) = guard.take() {
            h.abort();
            tracing::info!("org connector: aborted previous worker");
        }
    }

    let cfg = match crate::config::load() {
        Ok(c) => c.org_connector,
        Err(e) => {
            tracing::warn!(error = %e, "org connector respawn: config load failed; leaving worker stopped");
            return;
        }
    };

    let new_handle = spawn(storage, catalog, cfg, tail_kick);
    if new_handle.is_some() {
        tracing::info!("org connector: spawned fresh worker");
    } else {
        tracing::debug!("org connector: respawn produced no worker (disabled or misconfigured)");
    }
    *handle.lock() = new_handle;
}

/// How long a session must HOLD before it counts as evidence the connector
/// is healthy. Below this it is a flap, not a connection.
const STABLE_SESSION: Duration = Duration::from_secs(30);

/// Delay before the next connect attempt.
///
/// `session` is how long the last session lasted, or `None` when the
/// handshake itself failed.
///
/// THE BACKOFF USED TO RESET ON A SUCCESSFUL HANDSHAKE. That is not the same
/// as a successful session, and the difference is a hot loop: when the server
/// accepts the socket and immediately resets it without a closing handshake,
/// every attempt "connected", so every attempt reset the delay to
/// `BACKOFF_MIN` and the exponential backoff could never grow. Measured on one
/// install: 36,725 reconnects in a single day, one every 2.25 seconds, around
/// the clock — 2 seconds of `BACKOFF_MIN` plus the round trip — and 73,450 log
/// lines saying so.
///
/// The reset now needs a session that actually held. A server that keeps
/// dropping us is still a server-side fault, but the client's job is to stop
/// hammering it while that is true.
fn backoff_after_session(current: Duration, session: Option<Duration>) -> Duration {
    match session {
        Some(held) if held >= STABLE_SESSION => BACKOFF_MIN,
        _ => (current * 2).min(BACKOFF_MAX),
    }
}

/// Outer connect/reconnect loop. Each successful connection runs
/// [`pump`] until the socket closes or errors, then backs off and
/// reconnects. Telemetry is presence data — there's nothing to persist
/// across a disconnect beyond the event cursor (which lives in SQLite),
/// so a dropped connection just means "resume from the cursor on the
/// next connect".
async fn run_loop(
    ws_url: String,
    bearer_token: String,
    storage: Arc<Storage>,
    catalog: Arc<RwLock<Arc<LocationCatalog>>>,
    tail_kick: Arc<Notify>,
) {
    // Seed the cursor to the newest event on first run so enabling the
    // connector forwards only NEW presence, not a replay of the whole
    // event history (which would flood the org platform and is
    // meaningless as live presence). A resumed run (cursor already set)
    // keeps its place.
    seed_cursor_if_unset(&storage);

    let mut backoff = BACKOFF_MIN;
    let mut last_session: Option<Duration>;
    loop {
        // Build a fresh handshake request each attempt (connect_async
        // consumes it). The bearer token rides the `Authorization` header
        // — never the URL — so it can't leak into the platform's
        // reverse-proxy access logs. A `None` here is permanent (bad URL
        // or a token that isn't a valid header value), so we stop.
        let Some(request) = connect_request(&ws_url, &bearer_token) else {
            return;
        };
        match tokio_tungstenite::connect_async(request).await {
            Ok((socket, _resp)) => {
                tracing::info!("org connector connected");
                let started = tokio::time::Instant::now();
                if let Err(e) = pump(socket, &storage, &catalog, &tail_kick).await {
                    tracing::warn!(
                        error = %e,
                        held_secs = started.elapsed().as_secs(),
                        "org connector session ended"
                    );
                }
                // How long it HELD is what decides the next delay — see
                // `backoff_after_session`. Resetting here, on the handshake
                // alone, is what produced a 2-second reconnect loop that ran
                // for days.
                last_session = Some(started.elapsed());
            }
            Err(e) => {
                tracing::warn!(error = %e, backoff_secs = backoff.as_secs(), "org connector connect failed");
                last_session = None;
            }
        }
        backoff = backoff_after_session(backoff, last_session);
        tokio::time::sleep(backoff).await;
    }
}

/// One live session: forward new events the instant the tail kicks (or,
/// as a fallback, on a poll tick), drain (and ignore) inbound frames,
/// exit on close/error so the outer loop reconnects.
///
/// The `tail_kick` arm is what makes presence delivery immediate: the
/// Game.log tail fires it after every drain that ingested events, so a
/// zone change or death is forwarded within milliseconds instead of
/// waiting up to `POLL_INTERVAL`. The periodic `tick` stays as a safety
/// net — it catches any ingest path that doesn't fire the kick and
/// drives post-reconnect catch-up. Both arms call the same
/// cursor-driven `forward_new_events`, so an extra wake just reads
/// "nothing new" and returns; it can never double-send.
async fn pump<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    storage: &Storage,
    catalog: &RwLock<Arc<LocationCatalog>>,
    tail_kick: &Notify,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut tx, mut rx) = socket.split();
    let mut tick = tokio::time::interval(POLL_INTERVAL);

    loop {
        tokio::select! {
            _ = tick.tick() => {
                forward_new_events(storage, catalog, &mut tx).await?;
            }
            _ = tail_kick.notified() => {
                forward_new_events(storage, catalog, &mut tx).await?;
            }
            inbound = rx.next() => match inbound {
                // Server closed, or the stream ended — let the outer
                // loop reconnect.
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(e)) => return Err(e.into()),
                // OrgContext / Heartbeat / Ping(auto-pong) — not consumed
                // by this outbound-only connector.
                _ => {}
            },
        }
    }
}

/// Read events past the persisted cursor, map each to [`Telemetry`],
/// send the ones that carry signal, then advance the cursor past every
/// row we examined (including the no-signal ones, so we never re-scan
/// them). The cursor lives in SQLite, so it survives reconnects.
async fn forward_new_events<Sink>(
    storage: &Storage,
    catalog: &RwLock<Arc<LocationCatalog>>,
    tx: &mut Sink,
) -> anyhow::Result<()>
where
    Sink: SinkExt<Message> + Unpin,
    Sink::Error: std::error::Error + Send + Sync + 'static,
{
    let cursor = storage.read_cursor(CURSOR_KEY).unwrap_or(0) as i64;
    let rows = storage.read_events_after(cursor, BATCH)?;
    if rows.is_empty() {
        return Ok(());
    }

    // Snapshot the catalogue once (cheap Arc clone) so we don't hold the
    // RwLock guard across the awaiting send below.
    let cat = catalog.read().clone();

    let mut max_id = cursor;
    for row in &rows {
        max_id = max_id.max(row.id);
        let Ok(event) = serde_json::from_str::<GameEvent>(&row.payload_json) else {
            // A row we can't deserialise is one we'll never forward;
            // the cursor still advances past it via max_id.
            continue;
        };
        if let Some(telemetry) = telemetry_for(&event, &cat) {
            let json = serde_json::to_string(&OrgClientMessage::Telemetry(telemetry))?;
            tx.send(Message::Text(json)).await?;
        }
    }

    storage.write_cursor(CURSOR_KEY, max_id as u64)?;
    Ok(())
}

/// Seed the cursor to the newest stored event id when it's unset (0), so
/// a freshly-enabled connector forwards only events that arrive AFTER
/// it's switched on. Best-effort — a read failure just leaves the cursor
/// at 0, in which case the first poll forwards recent history once.
fn seed_cursor_if_unset(storage: &Storage) {
    if storage.read_cursor(CURSOR_KEY).unwrap_or(0) != 0 {
        return;
    }
    let latest = storage
        .recent_events(1)
        .ok()
        .and_then(|rows| rows.first().map(|r| r.id))
        .unwrap_or(0);
    if latest > 0 {
        if let Err(e) = storage.write_cursor(CURSOR_KEY, latest as u64) {
            tracing::warn!(error = %e, "org connector: failed to seed event cursor");
        }
    }
}

/// Project a parsed [`GameEvent`] down to the read-only [`Telemetry`] the
/// org platform understands, or `None` when the event carries no
/// presence signal worth forwarding.
///
/// Three signals, matching `hud-protocol::Telemetry`:
/// - `zone` — a friendly location label (via the shared classifier),
///   set only for events that genuinely indicate where the player IS
///   (not, e.g., a quantum *destination*).
/// - `quantum_state` — a coarse travel-state label.
/// - `event` — one of `spawn` | `kill` | `death` | `downed`.
fn telemetry_for(event: &GameEvent, catalog: &LocationCatalog) -> Option<Telemetry> {
    let zone = |raw: &str| classify(raw, catalog).display_name;

    let telemetry = match event {
        // Presence: where the player is now.
        GameEvent::LocationChanged(e) => Telemetry {
            zone: Some(zone(&e.to)),
            ..Default::default()
        },
        GameEvent::LocationInventoryRequested(e) => Telemetry {
            zone: Some(zone(&e.location)),
            ..Default::default()
        },
        GameEvent::PlanetTerrainLoad(e) => Telemetry {
            zone: Some(zone(&e.planet)),
            ..Default::default()
        },
        GameEvent::VehicleStowed(e) => Telemetry {
            zone: Some(zone(&e.landing_area)),
            ..Default::default()
        },

        // Travel state. The destination isn't where the player IS, so we
        // surface only the coarse state, not a zone.
        GameEvent::QuantumTargetSelected(e) => Telemetry {
            quantum_state: Some(
                match e.phase {
                    QuantumTargetPhase::Selected => "travelling",
                    QuantumTargetPhase::FuelRequested => "routing",
                }
                .to_string(),
            ),
            ..Default::default()
        },

        // Discrete combat / lifecycle markers.
        GameEvent::PlayerDeath(e) => Telemetry {
            zone: e.zone.as_deref().map(zone),
            event: Some("death".to_string()),
            ..Default::default()
        },
        GameEvent::ActorDeath(e) => Telemetry {
            zone: Some(zone(&e.zone)),
            event: Some("death".to_string()),
            ..Default::default()
        },
        GameEvent::PlayerIncapacitated(e) => Telemetry {
            zone: e.zone.as_deref().map(zone),
            event: Some("downed".to_string()),
            ..Default::default()
        },
        GameEvent::ResolveSpawn(_) => Telemetry {
            event: Some("spawn".to_string()),
            ..Default::default()
        },
        GameEvent::SeedSolarSystem(e) if e.success => Telemetry {
            zone: Some(zone(&e.solar_system)),
            event: Some("spawn".to_string()),
            ..Default::default()
        },

        _ => return None,
    };

    // Defensive: a match arm that produced an all-empty telemetry (e.g. a
    // future variant added above without a real field) is not worth a
    // frame.
    if telemetry == Telemetry::default() {
        return None;
    }
    Some(telemetry)
}

/// Build the WS handshake request, putting the bearer token in the
/// `Authorization` header rather than the URL so it never reaches the
/// platform's reverse-proxy access logs. Returns `None` on a malformed
/// URL or a token that can't be a header value — both permanent
/// conditions, so the caller stops instead of reconnect-looping.
fn connect_request(
    ws_url: &str,
    bearer_token: &str,
) -> Option<tokio_tungstenite::tungstenite::http::Request<()>> {
    let mut request = match ws_url.into_client_request() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "org connector: ws url is not a valid request");
            return None;
        }
    };
    let value = match HeaderValue::from_str(&format!("Bearer {bearer_token}")) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "org connector: bearer token is not a valid header value");
            return None;
        }
    };
    request.headers_mut().insert(AUTHORIZATION, value);
    Some(request)
}

/// Turn the user-configured `platform_url` into the member-channel
/// WebSocket URL (no token — that travels in the `Authorization` header).
///
/// Accepts `http(s)://`, `ws(s)://`, or a bare `host[:port]`:
/// - `http://` → `ws://`, `https://` → `wss://`; `ws://`/`wss://` kept.
/// - A scheme-less value defaults to `wss://`, except for a loopback host
///   (`localhost` / `::1` / `127.*`) where it defaults to `ws://`.
///
/// Plaintext `ws://` to a NON-loopback host is refused (`None` + a
/// warning): it would send the bearer token and presence in the clear.
/// Any existing path/query on the input is dropped — we always target
/// `/ws/member`. Returns `None` for an empty / unparseable input.
fn build_ws_url(platform_url: &str) -> Option<String> {
    let trimmed = platform_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    // Split an explicit scheme (mapped to its ws equivalent) from the
    // authority. A scheme-less value defers the scheme decision to the
    // loopback check below.
    let (explicit_scheme, rest): (Option<&str>, &str) = match trimmed.split_once("://") {
        Some(("http", rest)) => (Some("ws"), rest),
        Some(("https", rest)) => (Some("wss"), rest),
        Some(("ws", rest)) => (Some("ws"), rest),
        Some(("wss", rest)) => (Some("wss"), rest),
        Some(_) => return None,  // unknown scheme — don't guess
        None => (None, trimmed), // scheme-less → decide by loopback
    };

    // Keep only the authority (host[:port]); drop any path/query the user
    // pasted — the member channel is always at `/ws/member`.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }

    let host = host_of(authority);
    let is_loopback = host == "localhost" || host == "::1" || host.starts_with("127.");

    // Explicit scheme wins; scheme-less defaults to wss (TLS) for remote
    // hosts, ws for loopback so local test setups need no certs.
    let scheme = match explicit_scheme {
        Some(s) => s,
        None if is_loopback => "ws",
        None => "wss",
    };

    if scheme == "ws" && !is_loopback {
        tracing::warn!(
            host = %host,
            "org connector: refusing plaintext ws:// to a non-loopback host — use wss://"
        );
        return None;
    }

    Some(format!("{scheme}://{authority}/ws/member"))
}

/// Extract the host (without port) from an authority — handles `host`,
/// `host:port`, and the IPv6 bracket forms `[::1]` / `[::1]:port`.
fn host_of(authority: &str) -> &str {
    if let Some(after_bracket) = authority.strip_prefix('[') {
        // IPv6 literal: take everything up to the closing bracket.
        return after_bracket.split(']').next().unwrap_or(after_bracket);
    }
    authority.split(':').next().unwrap_or(authority)
}

#[cfg(test)]
mod tests {

    /// A server that accepts and instantly resets must NOT be hammered.
    ///
    /// This is the shape of the real fault: `connect_async` succeeds, `pump`
    /// returns an error within milliseconds, forever. The old code reset the
    /// delay on the handshake, so every one of those cycles went back to
    /// `BACKOFF_MIN` — 36,725 reconnects in a day on one install, one every
    /// 2.25 seconds, around the clock.
    ///
    /// Asserting only "the delay grows once" would pass on the old code too,
    /// because it did double — once — before the next handshake reset it. The
    /// load-bearing assertion is that it STAYS grown across many cycles.
    #[test]
    fn an_instantly_reset_session_escalates_and_stays_escalated() {
        let instant = Some(Duration::from_millis(20));
        let mut backoff = BACKOFF_MIN;
        let mut seen = Vec::new();
        for _ in 0..10 {
            backoff = backoff_after_session(backoff, instant);
            seen.push(backoff);
        }
        assert_eq!(
            seen.iter().filter(|d| **d == BACKOFF_MIN).count(),
            0,
            "a flapping session must never return to the floor; got {seen:?}",
        );
        assert_eq!(
            *seen.last().unwrap(),
            BACKOFF_MAX,
            "sustained flapping must settle at the ceiling; got {seen:?}",
        );
        assert!(
            seen.windows(2).all(|w| w[1] >= w[0]),
            "the delay must be monotonic while the fault persists; got {seen:?}",
        );
    }

    /// The other half: a connection that actually worked earns a fast retry.
    #[test]
    fn a_session_that_held_returns_to_the_floor() {
        let held = Some(STABLE_SESSION + Duration::from_secs(1));
        assert_eq!(backoff_after_session(BACKOFF_MAX, held), BACKOFF_MIN);
    }

    /// A failed handshake escalates like a flap — there is no session to judge.
    #[test]
    fn a_failed_handshake_escalates() {
        assert!(backoff_after_session(BACKOFF_MIN, None) > BACKOFF_MIN);
    }

    use super::*;
    use starstats_core::events::{
        ActorDeath, LocationChanged, PlayerDeath, PlayerIncapacitated, QuantumTargetSelected,
        ResolveSpawn, SeedSolarSystem,
    };

    fn empty_catalog() -> LocationCatalog {
        LocationCatalog::from_entries(vec![])
    }

    // ---- mapping ----------------------------------------------------

    #[test]
    fn location_changed_maps_to_a_zone() {
        let cat = empty_catalog();
        let ev = GameEvent::LocationChanged(LocationChanged {
            timestamp: "t".into(),
            from: None,
            to: "Stanton4_NewBabbage".into(),
        });
        let t = telemetry_for(&ev, &cat).expect("location event carries presence");
        assert!(t.zone.is_some(), "a location change must set a zone");
        assert!(t.event.is_none());
        assert!(t.quantum_state.is_none());
    }

    #[test]
    fn quantum_target_maps_to_travel_state_not_zone() {
        let cat = empty_catalog();
        let ev = GameEvent::QuantumTargetSelected(QuantumTargetSelected {
            timestamp: "t".into(),
            phase: QuantumTargetPhase::Selected,
            vehicle_class: "RSI_Constellation".into(),
            vehicle_id: "1".into(),
            destination: "OOC_Stanton_2_Crusader".into(),
        });
        let t = telemetry_for(&ev, &cat).expect("quantum target carries travel state");
        assert_eq!(t.quantum_state.as_deref(), Some("travelling"));
        // The destination is NOT where the player is — must not leak as a zone.
        assert!(
            t.zone.is_none(),
            "quantum destination must not become a zone"
        );
    }

    #[test]
    fn deaths_and_spawns_map_to_discrete_markers() {
        let cat = empty_catalog();

        let death = telemetry_for(
            &GameEvent::PlayerDeath(PlayerDeath {
                timestamp: "t".into(),
                body_class: "body_01".into(),
                body_id: "1".into(),
                zone: Some("Stanton2_Orison".into()),
            }),
            &cat,
        )
        .unwrap();
        assert_eq!(death.event.as_deref(), Some("death"));
        assert!(death.zone.is_some(), "death carries its zone when known");

        let downed = telemetry_for(
            &GameEvent::PlayerIncapacitated(PlayerIncapacitated {
                timestamp: "t".into(),
                queue_id: 7,
                zone: None,
            }),
            &cat,
        )
        .unwrap();
        assert_eq!(downed.event.as_deref(), Some("downed"));

        let actor = telemetry_for(
            &GameEvent::ActorDeath(ActorDeath {
                timestamp: "t".into(),
                victim: "v".into(),
                victim_geid: None,
                zone: "Stanton2_Orison".into(),
                killer: "k".into(),
                killer_geid: None,
                weapon: "w".into(),
                damage_type: "d".into(),
            }),
            &cat,
        )
        .unwrap();
        assert_eq!(actor.event.as_deref(), Some("death"));

        let spawn = telemetry_for(
            &GameEvent::ResolveSpawn(ResolveSpawn {
                timestamp: "t".into(),
                player_geid: "1".into(),
                fallback: true,
            }),
            &cat,
        )
        .unwrap();
        assert_eq!(spawn.event.as_deref(), Some("spawn"));
    }

    #[test]
    fn failed_seed_solar_system_is_not_a_spawn() {
        let cat = empty_catalog();
        let ev = GameEvent::SeedSolarSystem(SeedSolarSystem {
            timestamp: "t".into(),
            solar_system: "Stanton".into(),
            shard: "s".into(),
            success: false,
        });
        assert!(
            telemetry_for(&ev, &cat).is_none(),
            "a failed seed isn't a spawn — no telemetry"
        );
    }

    #[test]
    fn placeless_events_produce_no_telemetry() {
        let cat = empty_catalog();
        // SessionEnd carries no presence signal.
        let ev = GameEvent::SessionEnd(starstats_core::events::SessionEnd {
            timestamp: "t".into(),
            kind: starstats_core::events::SessionEndKind::SystemQuit,
        });
        assert!(telemetry_for(&ev, &cat).is_none());
    }

    // ---- wire shape -------------------------------------------------

    #[test]
    fn telemetry_serialises_to_the_hud_protocol_wire_shape() {
        // Must be byte-compatible with hud-protocol's
        // `ClientMessage::Telemetry` arm: an internally-tagged object
        // `{"type":"telemetry", ...}` with zone/quantum_state/event.
        let msg = OrgClientMessage::Telemetry(Telemetry {
            zone: Some("Daymar".into()),
            quantum_state: None,
            event: Some("kill".into()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"telemetry\""), "got: {json}");
        assert!(json.contains("\"zone\":\"Daymar\""), "got: {json}");
        assert!(json.contains("\"event\":\"kill\""), "got: {json}");
        // None fields are skipped to keep frames small (wire-compatible:
        // serde reads a missing Option back as None server-side).
        assert!(!json.contains("quantum_state"), "got: {json}");
    }

    // ---- url building -----------------------------------------------

    #[test]
    fn build_ws_url_maps_https_to_wss_and_strips_path() {
        assert_eq!(
            build_ws_url("https://orgs.example").as_deref(),
            Some("wss://orgs.example/ws/member")
        );
        // A pasted path/trailing slash is discarded — we always target /ws/member.
        assert_eq!(
            build_ws_url("https://orgs.example/some/path/").as_deref(),
            Some("wss://orgs.example/ws/member")
        );
    }

    #[test]
    fn build_ws_url_refuses_plaintext_to_remote() {
        // http → ws to a remote host would send the token in the clear.
        assert!(build_ws_url("http://orgs.example:8080").is_none());
        // Explicit ws:// to a remote host is refused for the same reason.
        assert!(build_ws_url("ws://orgs.example").is_none());
    }

    #[test]
    fn build_ws_url_allows_plaintext_ws_for_loopback() {
        assert_eq!(
            build_ws_url("ws://localhost:3000").as_deref(),
            Some("ws://localhost:3000/ws/member")
        );
        // http → ws is fine when the host is loopback.
        assert_eq!(
            build_ws_url("http://127.0.0.1:8080").as_deref(),
            Some("ws://127.0.0.1:8080/ws/member")
        );
        // IPv6 loopback literal.
        assert_eq!(
            build_ws_url("ws://[::1]:9000").as_deref(),
            Some("ws://[::1]:9000/ws/member")
        );
    }

    #[test]
    fn build_ws_url_schemeless_defaults_to_wss_remote_ws_loopback() {
        // Scheme-less remote → wss (TLS required).
        assert_eq!(
            build_ws_url("orgs.example:9000").as_deref(),
            Some("wss://orgs.example:9000/ws/member")
        );
        // Scheme-less loopback → ws (no certs needed locally).
        assert_eq!(
            build_ws_url("localhost:3000").as_deref(),
            Some("ws://localhost:3000/ws/member")
        );
    }

    #[test]
    fn build_ws_url_rejects_empty_or_unknown_scheme() {
        assert!(build_ws_url("").is_none());
        assert!(build_ws_url("   ").is_none());
        assert!(build_ws_url("ftp://orgs.example").is_none());
    }

    // ---- spawn / respawn --------------------------------------------

    fn make_storage() -> (Arc<crate::storage::Storage>, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("test.sqlite3");
        let storage = crate::storage::Storage::open(&path).expect("open storage");
        (Arc::new(storage), dir)
    }

    fn make_catalog() -> Arc<parking_lot::RwLock<Arc<LocationCatalog>>> {
        Arc::new(parking_lot::RwLock::new(Arc::new(empty_catalog())))
    }

    // ---- forwarding pipeline ----------------------------------------

    /// A minimal in-memory [`futures_util::Sink`] that records every frame
    /// pushed to it, so we can assert exactly what the connector would put
    /// on the wire without standing up a real WebSocket.
    #[derive(Clone, Default)]
    struct CollectSink(std::rc::Rc<std::cell::RefCell<Vec<Message>>>);

    impl futures_util::Sink<Message> for CollectSink {
        type Error = std::convert::Infallible;
        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn start_send(self: std::pin::Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.0.borrow_mut().push(item);
            Ok(())
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    fn frame_bodies(sink: &CollectSink) -> Vec<String> {
        sink.0
            .borrow()
            .iter()
            .map(|m| match m {
                Message::Text(s) => s.clone(),
                other => panic!("expected a text frame, got {other:?}"),
            })
            .collect()
    }

    /// The end-to-end local hop the tail kick triggers: events sitting in
    /// SQLite are projected to telemetry frames, put on the wire, and the
    /// cursor advances so a re-poll re-sends nothing. This is what runs on
    /// every kick AND every fallback tick — proving it's idempotent across
    /// wakes is the whole safety argument for "kick + poll".
    #[tokio::test]
    async fn forward_new_events_emits_one_frame_per_signal_then_advances_cursor() {
        let (storage, _dir) = make_storage();
        let catalog = make_catalog();

        // Two presence-bearing events land locally, as the tail writes
        // them: a zone change (location) and a death (life status).
        let loc = serde_json::to_string(&GameEvent::LocationChanged(LocationChanged {
            timestamp: "t1".into(),
            from: None,
            to: "Stanton4_NewBabbage".into(),
        }))
        .unwrap();
        let death = serde_json::to_string(&GameEvent::PlayerDeath(PlayerDeath {
            timestamp: "t2".into(),
            body_class: "body_01".into(),
            body_id: "1".into(),
            zone: Some("Stanton2_Orison".into()),
        }))
        .unwrap();
        storage
            .insert_event("k1", "location_changed", "t1", "raw1", &loc, "live", 0)
            .unwrap();
        storage
            .insert_event("k2", "player_death", "t2", "raw2", &death, "live", 1)
            .unwrap();

        // Capture everything the connector would push to the org platform.
        let sink = CollectSink::default();
        let mut tx = sink.clone();
        forward_new_events(&storage, &catalog, &mut tx)
            .await
            .expect("forward succeeds");

        let bodies = frame_bodies(&sink);
        assert_eq!(bodies.len(), 2, "one frame per presence-bearing event");
        assert!(
            bodies.iter().any(|b| b.contains("\"zone\"")),
            "the location change must carry a zone: {bodies:?}"
        );
        assert!(
            bodies.iter().any(|b| b.contains("\"event\":\"death\"")),
            "the death must carry a life-status marker: {bodies:?}"
        );

        // Cursor advanced past both rows — a second wake re-sends nothing.
        assert!(
            storage.read_cursor(CURSOR_KEY).unwrap_or(0) >= 2,
            "cursor must advance past every examined row"
        );
        let sink2 = CollectSink::default();
        let mut tx2 = sink2.clone();
        forward_new_events(&storage, &catalog, &mut tx2)
            .await
            .expect("second forward succeeds");
        assert!(
            frame_bodies(&sink2).is_empty(),
            "a wake with no new events must not re-send"
        );
    }

    #[test]
    fn spawn_returns_none_when_disabled() {
        let (storage, _dir) = make_storage();
        let catalog = make_catalog();
        let cfg = OrgConnectorConfig {
            enabled: false,
            platform_url: Some("https://orgs.example".into()),
            bearer_token: Some("tok".into()),
        };
        assert!(
            spawn(storage, catalog, cfg, Arc::new(Notify::new())).is_none(),
            "disabled connector must not spawn a task"
        );
    }

    #[test]
    fn spawn_returns_none_when_fields_missing() {
        let (storage, _dir) = make_storage();
        let catalog = make_catalog();
        let cfg = OrgConnectorConfig {
            enabled: true,
            platform_url: None,
            bearer_token: None,
        };
        assert!(
            spawn(storage, catalog, cfg, Arc::new(Notify::new())).is_none(),
            "connector with missing url/token must not spawn"
        );
    }

    #[test]
    fn spawn_returns_none_when_url_is_bad_scheme() {
        let (storage, _dir) = make_storage();
        let catalog = make_catalog();
        let cfg = OrgConnectorConfig {
            enabled: true,
            platform_url: Some("ftp://orgs.example".into()),
            bearer_token: Some("tok".into()),
        };
        assert!(
            spawn(storage, catalog, cfg, Arc::new(Notify::new())).is_none(),
            "connector with unsupported URL scheme must not spawn"
        );
    }

    #[tokio::test]
    async fn spawn_returns_some_and_respawn_aborts_then_clears() {
        let (storage, _dir) = make_storage();
        let catalog = make_catalog();
        let cfg = OrgConnectorConfig {
            enabled: true,
            platform_url: Some("wss://orgs.example".into()),
            bearer_token: Some("tok".into()),
        };
        // Enabled config with valid URL → handle is Some
        let handle = spawn(
            Arc::clone(&storage),
            Arc::clone(&catalog),
            cfg,
            Arc::new(Notify::new()),
        );
        assert!(
            handle.is_some(),
            "enabled + valid config must produce a handle"
        );

        // Wrap in the Arc<Mutex<Option<…>>> shape that respawn uses.
        let slot = Arc::new(parking_lot::Mutex::new(handle));

        // Aborting the slot should leave it Some initially (abort is
        // non-blocking), but after locking and taking, it becomes None.
        {
            let mut guard = slot.lock();
            if let Some(h) = guard.take() {
                h.abort();
            }
            assert!(guard.is_none(), "take() must clear the slot");
        }
        assert!(slot.lock().is_none(), "slot is clear after abort+take");

        // respawn with disabled config → slot stays None
        let disabled_cfg = OrgConnectorConfig::default(); // enabled = false
        let new_handle = spawn(
            Arc::clone(&storage),
            Arc::clone(&catalog),
            disabled_cfg,
            Arc::new(Notify::new()),
        );
        assert!(new_handle.is_none(), "disabled config leaves handle None");
        *slot.lock() = new_handle;
        assert!(
            slot.lock().is_none(),
            "slot remains None for disabled config"
        );
    }
}
