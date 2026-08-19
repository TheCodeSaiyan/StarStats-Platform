//! Two-pass `Game.log` parser.
//!
//! ## Why two passes
//!
//! Game.log lines have a stable shell:
//!
//! ```text
//! <TIMESTAMP> [LEVEL] <EventName> body fields [Team_X][Y][Z]
//! ```
//!
//! and a wildly variable body. Trying to capture every event with one
//! big regex per type is brittle: CIG changes wording between patches,
//! and a single broken regex hides the rest.
//!
//! Pass 1 ([`structural_parse`]) extracts the stable shell into a
//! [`LogLine`]. Pass 2 ([`classify`]) dispatches on `(event_name, tags)`
//! and runs a small, focused regex per event variant. Unknown lines
//! are recorded in [`ParseStats`] so we can surface coverage to the
//! user.

use crate::events::{
    ActorDeath, AttachmentReceived, ChangeServer, CommodityBuyRequest, CommoditySellRequest,
    EquipAction, GameEvent, HudNotification, ItemEquipChange, JoinPu, LauncherCategory,
    LegacyLogin, LocationInventoryRequested, MissionEnd, MissionMarkerKind, MissionObjective,
    MissionObjectiveState, MissionQuantumDestinationSelected, MissionStart, PlanetTerrainLoad,
    PlayerDeath, PlayerIncapacitated, ProcessInit, QuantumArrived, QuantumRoute,
    QuantumTargetPhase, QuantumTargetSelected, ResolveSpawn, SeedSolarSystem, ServerPhase,
    SessionEnd, SessionEndKind, ShopBuyRequest, ShopFlowResponse, VehicleDestruction,
    VehicleStowed,
};
use once_cell::sync::Lazy;
use regex::Regex;

// ---------------------------------------------------------------------
// RSI Launcher log structural parser (separate format from Game.log)
// ---------------------------------------------------------------------

/// One line from an RSI Launcher log, decomposed for ingest.
///
/// The launcher writes Electron-style entries:
///
///   `[2026-05-06 12:34:56.789] [info] message text`
///
/// We only carry the structural shell — `timestamp`, `level`, and
/// `message`. Higher-level classification (login, install, patch
/// progress) is intentionally deferred until we have sample lines
/// to anchor regexes against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherLogLine<'a> {
    /// The bracketed date+time. Returned as-is (no normalisation) so
    /// the caller can decide whether to parse it. Format observed in
    /// the wild is `YYYY-MM-DD HH:MM:SS.mmm`.
    pub timestamp: &'a str,
    /// Lowercased level token (`info`, `warn`, `error`, `debug`, ...).
    pub level: &'a str,
    /// Trimmed message body — everything after the second `]`.
    pub message: &'a str,
}

static LAUNCHER_RE: Lazy<Regex> = Lazy::new(|| {
    // [TS] [LEVEL] message
    // Allow any non-`]` characters inside both bracket groups so a
    // future launcher version that pads the timestamp differently
    // still matches.
    Regex::new(r"^\[(?P<ts>[^\]]+)\]\s*\[(?P<level>[A-Za-z]+)\]\s*(?P<msg>.*)$")
        .expect("LAUNCHER_RE compiles")
});

/// Bucket a launcher message into a coarse [`LauncherCategory`] using
/// keyword detection. The launcher's vocabulary isn't formally
/// documented but real captures cluster around the same handful of
/// activities — auth, install, patch, update — each signalled by
/// predictable substrings. We err on the side of `Info` when the
/// signal is weak; a wrongly-categorised launcher message is far
/// less harmful than a missed gameplay event.
///
/// `level` and `message` come from [`parse_launcher_line`] (level is
/// pre-lowercased by the caller).
pub fn classify_launcher_message(level: &str, message: &str) -> LauncherCategory {
    // Lower-case once so the keyword scan is case-insensitive without
    // allocating per-keyword.
    let lower = message.to_ascii_lowercase();

    // Errors are level-driven first — explicit error level outranks
    // any keyword match (a "patch download error" is an Error, not a
    // Patch). Some lines log at info level but carry "failed" /
    // "error" in the body; treat those as errors too.
    let level_is_error = matches!(level, "error" | "fatal");
    let body_says_error = lower.contains("error")
        || lower.contains("failed")
        || lower.contains("failure")
        || lower.contains("exception")
        || lower.contains("err_");
    if level_is_error || body_says_error {
        return LauncherCategory::Error;
    }

    // Auth — login flow, session, credentials. RSI launcher uses
    // both "log in" and "sign in" wording across versions.
    if lower.contains("log in")
        || lower.contains("logged in")
        || lower.contains("logging in")
        || lower.contains("log out")
        || lower.contains("logged out")
        || lower.contains("sign in")
        || lower.contains("signed in")
        || lower.contains("authenticat")
        || lower.contains("credential")
        || lower.contains("session refresh")
        || lower.contains("token refresh")
        || lower.contains("oauth")
    {
        return LauncherCategory::Auth;
    }

    // Patch keywords are checked before Install because "applying
    // patch" should win over a stray "install" mention nearby. Patch
    // operations include explicit patch verbs plus delta downloads.
    if lower.contains("patch")
        || lower.contains("delta")
        || lower.contains("applying update")
        || lower.contains("applying ")
    {
        return LauncherCategory::Patch;
    }

    // Install / verify covers fresh installs, file integrity checks,
    // re-downloads. The launcher uses "verify" for the integrity pass.
    if lower.contains("install")
        || lower.contains("download")
        || lower.contains("verify")
        || lower.contains("verification")
        || lower.contains("integrity")
        || lower.contains("repair")
        || lower.contains("file check")
    {
        return LauncherCategory::Install;
    }

    // Self-update of the launcher itself.
    if lower.contains("launcher update")
        || lower.contains("self-update")
        || lower.contains("self update")
        || lower.contains("checking for updates")
    {
        return LauncherCategory::Update;
    }

    LauncherCategory::Info
}

/// Parse a single line from an RSI Launcher log. Returns `None` for
/// blanks, banners, multi-line continuations, or anything that
/// doesn't carry the bracketed timestamp + level shell.
pub fn parse_launcher_line(line: &str) -> Option<LauncherLogLine<'_>> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return None;
    }
    let caps = LAUNCHER_RE.captures(line)?;
    let timestamp = caps.name("ts")?.as_str().trim();
    let level = caps.name("level")?.as_str();
    let message = caps.name("msg")?.as_str().trim();
    // The launcher emits some lines that are pure level-only chatter
    // ("[info]") with no message body. Drop them — they carry no
    // signal worth surfacing on the timeline.
    if message.is_empty() {
        return None;
    }
    Some(LauncherLogLine {
        timestamp,
        level,
        message,
    })
}

// ---------------------------------------------------------------------
// Pass 1: structural pre-parse
// ---------------------------------------------------------------------

/// One Game.log line, decomposed into its stable parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine<'a> {
    pub timestamp: &'a str,
    pub level: Option<&'a str>,
    pub event_name: Option<&'a str>,
    pub body: &'a str,
    pub tags: Vec<&'a str>,
}

static SHELL_RE: Lazy<Regex> = Lazy::new(|| {
    // <TS> [LEVEL] <Event> ... [tag1][tag2]...
    // The tag run at the end is greedy from the LAST `]` walking back —
    // we strip it in code rather than via regex, for robustness.
    //
    // Event name allows ONE level of nested `<...>` so symbols like
    // `<CSCLoadingPlatformManager::LoadEntitiesReference::<lambda_1>>`
    // are captured whole instead of being truncated at the inner `>`.
    Regex::new(r"^<(?P<ts>[^>]+)>\s+(?:\[(?P<level>[A-Za-z]+)\]\s+)?(?:<(?P<event>(?:[^<>]|<[^>]*>)+)>)?\s*(?P<rest>.*)$")
        .expect("SHELL_RE compiles")
});

static TAG_TAIL_RE: Lazy<Regex> = Lazy::new(|| {
    // Trailing block: one or more [Team_X] / [Foo] tokens at end of line.
    Regex::new(r"\s*((?:\[[A-Za-z0-9_]+\])+)\s*$").expect("TAG_TAIL_RE compiles")
});

/// Parse the structural shell of one log line. Returns `None` for
/// lines that don't even start with a timestamp (continuation lines,
/// blanks, banners).
pub fn structural_parse(line: &str) -> Option<LogLine<'_>> {
    let line = line.trim_end_matches(['\r', '\n']);
    let caps = SHELL_RE.captures(line)?;
    let timestamp = caps.name("ts")?.as_str();
    let level = caps.name("level").map(|m| m.as_str());
    let event_name = caps.name("event").map(|m| m.as_str());
    let rest = caps.name("rest").map(|m| m.as_str()).unwrap_or("");

    // Walk the trailing tag block off `rest`.
    let (body, tags) = if let Some(tag_caps) = TAG_TAIL_RE.captures(rest) {
        let tag_run = tag_caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let body = &rest[..rest.len() - tag_caps.get(0).unwrap().as_str().len()];
        let tags = tag_run
            .split(['[', ']'])
            .filter(|s| !s.is_empty())
            .collect();
        (body.trim(), tags)
    } else {
        (rest.trim(), Vec::new())
    };

    Some(LogLine {
        timestamp,
        level,
        event_name,
        body,
        tags,
    })
}

// ---------------------------------------------------------------------
// Pass 2: semantic classification
// ---------------------------------------------------------------------

static PROCESS_INIT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"Process sc-client started\s*\(Local:\s*(?P<local>[^.]+)\.\s*Env:\s*(?P<env>[^)]+)\).*?bOnline\[(?P<online>\d)\]"
    ).expect("PROCESS_INIT_RE compiles")
});

static LEGACY_LOGIN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"User Login Success\s*-\s*Handle\[(?P<handle>[^\]]+)\](?:\s*-\s*Time\[(?P<time>[^\]]+)\])?")
        .expect("LEGACY_LOGIN_RE compiles")
});

static JOIN_PU_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"address\[(?P<address>[^\]]+)\]\s*port\[(?P<port>\d+)\]\s*shard\[(?P<shard>[^\]]+)\]\s*locationId\[(?P<loc>[^\]]+)\]"
    ).expect("JOIN_PU_RE compiles")
});

static CHANGE_SERVER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"IsShardPersisted\[(?P<shard>\d)\]\s*IsServer\[(?P<server>\d)\]\s*IsMultiplayer\[(?P<mp>\d)\](?:\s*IsOnline\[(?P<online>\d)\])?"
    ).expect("CHANGE_SERVER_RE compiles")
});

static SEED_SS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"Solar System '(?P<ss>[^']+)' for shard (?P<shard>\S+)")
        .expect("SEED_SS_RE compiles")
});

static RESOLVE_SPAWN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"player id:\s*\[(?P<geid>\d+)\]").expect("RESOLVE_SPAWN_RE compiles"));

// Combat events — patterns derived from community captures, NOT this
// fixture (which has no combat). Kept in their own block so they're
// easy to update when we get a real combat capture.
static ACTOR_DEATH_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?:CActor::Kill:\s*)?'(?P<victim>[^']+)'(?:\s*\[(?P<vgeid>\d+)\])?\s*in zone\s*'(?P<zone>[^']+)'\s*killed by\s*'(?P<killer>[^']+)'(?:\s*\[(?P<kgeid>\d+)\])?\s*using\s*'(?P<weapon>[^']+)'.*?with damage type\s*'(?P<dmg>[^']+)'"
    ).expect("ACTOR_DEATH_RE compiles")
});

static VEHICLE_DESTRUCTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"vehicle\s*'(?P<vclass>[^']+)'(?:\s*\[(?P<vid>[^\]]+)\])?.*?destroy(?:Level|_level)\s*[:=]?\s*(?P<level>\d+).*?caused by\s*'(?P<cause>[^']+)'(?:.*?zone\s*'(?P<zone>[^']+)')?"
    ).expect("VEHICLE_DESTRUCTION_RE compiles")
});

// Modern (4.x+) player-death signal. The corpse-cleanup burst starts
// with the player's body component being marked for inventory recovery
// — that single line is the death event. Subsequent items in the same
// burst (armor, weapons, mags) are ignored: they don't start with
// `body_` so the regex won't match.
//
// Anchoring on the leading `Item 'body_` keeps us from misclassifying
// equipment-cleanup lines that share the same event_name. The body_id
// is the trailing instance number on the body item, which also appears
// later as `KeptId`.
static PLAYER_DEATH_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^Item\s*'(?P<body_class>body_[A-Za-z0-9_]+?)_(?P<body_id>\d+)\s*-\s*Class\(body_[A-Za-z0-9_]+\)"
    ).expect("PLAYER_DEATH_RE compiles")
});

// HUD banner notifications (zone/jurisdiction/armistice). The text is
// allowed to contain a trailing colon-space because the engine appends
// a player or location after the colon; we keep what's inside the
// quotes verbatim so callers can detect that suffix themselves.
static HUD_NOTIFICATION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"Added notification\s*"(?P<text>[^"]*)"\s*\[(?P<id>\d+)\](?:.*?MissionId:\s*\[(?P<mission>[^\]]+)\])?(?:.*?ObjectiveId:\s*\[(?P<objective>[^\]]*)\])?"#
    ).expect("HUD_NOTIFICATION_RE compiles")
});

static LOCATION_INVENTORY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"Player\[(?P<player>[^\]]+)\]\s*requested\s*inventory\s*for\s*Location\[(?P<location>[^\]]+)\]"
    ).expect("LOCATION_INVENTORY_RE compiles")
});

// Match `Planet OOC_<...>` up to whitespace — the object-container
// key never has spaces in it.
static PLANET_TERRAIN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Planet\s+(?P<planet>OOC_\S+)").expect("PLANET_TERRAIN_RE compiles"));

// Quantum target lines look like:
//   ... | NOT AUTH | <vehicle_class>_<vehicle_id>[<vehicle_id>]| ... |Player has (selected point|requested fuel calculation to destination) <destination>...
// The vehicle field uses an underscore between class and id, but the
// id is also re-emitted in `[brackets]` so we anchor on that.
static QUANTUM_TARGET_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"\|\s*(?P<vehicle>[A-Za-z0-9_]+?)_(?P<vid>\d+)\[(?P<vid2>\d+)\]\|.*?Player has (?:selected point|requested fuel calculation to destination)\s+(?P<dest>\S+)"
    ).expect("QUANTUM_TARGET_RE compiles")
});

// Calculate Route lines look like:
//   ... | NOT AUTH | <vehicle_class>_<vehicle_id>[<vehicle_id>]|CSCItemNavigation::CalculateRoute|Projected Start Location is <start> for route to destination <dest> ...
// `dest` is often a transient `PartyMemberMarker_<id>` rather than a
// catalogued place -- see `QuantumRoute`'s doc comment for why this is
// deliberately not wired into `location_raw`.
// Quantum arrival lines carry the vehicle but NO destination:
//   ... | ARGO_MOLE_662886645202[662886645202]|CSCItemNavigation::OnQuantumDriveArrived|Quantum Drive has arrived at final destination ...
// Anchored on the method name so a future "arrived at waypoint" line
// cannot silently classify as a completed trip.
static QUANTUM_ARRIVED_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?P<vehicle>[A-Za-z0-9_]+?)_(?P<vid>\d+)\[(?P<vid2>\d+)\]\|CSCItemNavigation::OnQuantumDriveArrived\|",
    )
    .expect("QUANTUM_ARRIVED_RE compiles")
});

static CALCULATE_ROUTE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<vehicle>[A-Za-z0-9_]+?)_(?P<vid>\d+)\[(?P<vid2>\d+)\]\|CSCItemNavigation::CalculateRoute\|Projected Start Location is (?P<start>\S+) for route to destination (?P<dest>\S+)")
        .expect("CALCULATE_ROUTE_RE compiles")
});

// A quantum-target destination that is a mission / contract beacon
// (`MISSION_QT_Quantum_Beacon_<id>`) rather than a catalogued place.
// Anchored end-to-end so only the exact beacon form matches — a normal
// destination (`OOC_Stanton_2_Crusader`, `Orison_LOC`) never does. Used
// to divert the mission form of `<Player Selected Quantum Target -
// Local>` into `MissionQuantumDestinationSelected` so the same line
// doesn't ALSO emit a generic `QuantumTargetSelected` (single-event-per-
// line: the branch is inside the one shared dispatch arm).
static MISSION_QT_BEACON_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^MISSION_QT_Quantum_Beacon_(?P<beacon>\d+)$")
        .expect("MISSION_QT_BEACON_RE compiles")
});

// Landing-area unregister body:
//   [STOWING ON UNREGISTER] <area_name> [<area_id>] - Attempting to stow current vehicle [<vehicle_id>] ... Vehicle Zone Host [<zone_id>]
// `area_name` is whitespace-free even when prefixed with `[PROC]` and
// suffixed with a GUID-in-braces, so a single \S+ captures it cleanly.
// Anchoring the last capture at `Vehicle Zone Host [N]` (rather than at
// end-of-line) is what let the 2026-07 patch append
// `, IsAuthorityPossible [N]` without breaking the match.
static VEHICLE_STOWED_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"\[STOWING\s+ON\s+UNREGISTER\]\s*(?P<area>\S+)\s*\[(?P<area_id>\d+)\]\s*-\s*Attempting to stow current vehicle\s*\[(?P<vehicle_id>\d+)\].*?Vehicle Zone Host\s*\[(?P<zone_host>\d+)\]"
    ).expect("VEHICLE_STOWED_RE compiles")
});

// AttachmentReceived body shape:
//   Player[NAME] Attachment[name_with_id, class, id] Status[X] Port[Y] Elapsed[N.NNN]
// The Attachment[] tuple's first field repeats class+"_"+id, so we
// drop it and capture only the canonical class + id.
static ATTACHMENT_RECEIVED_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"Player\[(?P<player>[^\]]+)\]\s*Attachment\[[^,]+,\s*(?P<class>[^,]+?),\s*(?P<id>[^\]]+)\]\s*Status\[(?P<status>[^\]]+)\]\s*Port\[(?P<port>[^\]]+)\]\s*Elapsed\[(?P<elapsed>[^\]]+)\]"
    ).expect("ATTACHMENT_RECEIVED_RE compiles")
});

// `<EquipItem>` body shape:
//   ... Class[<class>] Rank[...] Port[<port>] ...
// `<StoreItem>` body shape:
//   ... Class[<class>] Rank[...] ItemsCount[<n>] ...
static EQUIP_ITEM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"Class\[(?P<class>[^\]]+)\].*?Port\[(?P<port>[^\]]+)\]")
        .expect("EQUIP_ITEM_RE compiles")
});
static STORE_ITEM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"Class\[(?P<class>[^\]]+)\].*?ItemsCount\[(?P<count>\d+)\]")
        .expect("STORE_ITEM_RE compiles")
});

// ---------------------------------------------------------------------
// v0.2.0-alpha additions: mission / shop / commodity / session-end.
//
// Patterns derived from external community reverse-engineering work
// (see repo NOTICE). They are best-effort: real captures may force
// tightening.  Every regex is anchored on its keyword so substring
// matches in unrelated lines won't false-positive.
// ---------------------------------------------------------------------

// `<CLocalMissionPhaseMarker::CreateMarker>` body shape (best-effort):
//   ... missionId[<uuid>] ... missionName[<name>]
static MISSION_PHASE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"missionId\[(?P<id>[0-9a-fA-F-]+)\](?:.*?missionName\[(?P<name>[^\]]+)\])?")
        .expect("MISSION_PHASE_RE compiles")
});

// `CreateMissionObjectiveMarker(...)` function-call body. The args
// list typically contains the parent mission UUID and a localised
// objective string we don't try to translate. We capture only the UUID.
static MISSION_OBJECTIVE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"CreateMissionObjectiveMarker\([^)]*?(?P<id>[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})"
    ).expect("MISSION_OBJECTIVE_RE compiles")
});

// `<EndMission>` body — typically `... missionId[<uuid>] outcome[<word>] ...`.
// Outcome is optional because the engine sometimes prints just the id.
static MISSION_END_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:missionId\[(?P<id>[0-9a-fA-F-]+)\])?(?:.*?outcome\[(?P<outcome>[A-Za-z]+)\])?")
        .expect("MISSION_END_RE compiles")
});

// Shop / standard-item buy request — function-call style. Body looks like:
//   `Send(?:Standard)?ShopBuyRequest(shopId=..., item=..., quantity=N, ...)`
// We only require the keyword anchor; the inner KV list varies.
static SHOP_BUY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"Send(?:Standard)?(?:Item|Shop)?BuyRequest(?:.*?shopId=(?P<shop>[A-Za-z0-9_-]+))?(?:.*?item(?:Class)?=(?P<item>[A-Za-z0-9_]+))?(?:.*?quantity=(?P<qty>\d+))?"
    ).expect("SHOP_BUY_RE compiles")
});

// Modern kiosk buy line (shell-tagged `<...SendStandardItemBuyRequest>` /
// `<...SendShopBuyRequest>`): bracketed fields incl. shopName + client_price.
static SHOP_BUY_MODERN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"shopId\[(?P<shop>[^\]]+)\].*?shopName\[(?P<shopname>[^\]]+)\].*?client_price\[(?P<price>[0-9.]+)\](?:.*?itemClassGUID\[(?P<item>[^\]]+)\])?"
    ).expect("SHOP_BUY_MODERN_RE compiles")
});

// Server-side flow response — keyword anchor + optional Result token.
static SHOP_RESPONSE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"ShopFlowResponse(?:.*?shopId=(?P<shop>[A-Za-z0-9_-]+))?(?:.*?Result\[(?P<result>[A-Za-z]+)\])?"
    ).expect("SHOP_RESPONSE_RE compiles")
});

static COMMODITY_BUY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"SendCommodityBuyRequest(?:.*?commodity=(?P<commodity>[A-Za-z0-9_]+))?(?:.*?(?:quantity|amount)=(?P<qty>[0-9.]+))?"
    ).expect("COMMODITY_BUY_RE compiles")
});

static COMMODITY_SELL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"SendCommoditySellRequest(?:.*?commodity=(?P<commodity>[A-Za-z0-9_]+))?(?:.*?(?:quantity|amount)=(?P<qty>[0-9.]+))?"
    ).expect("COMMODITY_SELL_RE compiles")
});

// `<ObjectiveUpserted>` push body:
//   Received ObjectiveUpserted push message for: mission_id <uuid> -
//   objective_id <uuid> - state MISSION_OBJECTIVE_STATE_<WORD> - created 0 - flags=...
static OBJECTIVE_UPSERTED_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"mission_id\s+(?P<mission>[0-9a-fA-F-]+)\s+-\s+objective_id\s+(?P<obj>[0-9a-fA-F-]+)\s+-\s+state\s+MISSION_OBJECTIVE_STATE_(?P<state>\w+)")
        .expect("OBJECTIVE_UPSERTED_RE compiles")
});

// `<CMissionLogEntry::UpdateActiveObjective>` body:
//   Objective updated id=<uuid>, flags=..., hidden=0, hiddenInUI=0,
//   markerHidden=0, uiDisplay[Priority=N][Text=<verbatim, often a template>]
static OBJECTIVE_LABEL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"id=(?P<obj>[0-9a-fA-F-]+).*?\[Text=(?P<text>[^\]]*)\]")
        .expect("OBJECTIVE_LABEL_RE compiles")
});

/// Tally of recognised vs unrecognised lines, useful for surfacing
/// parser coverage to the user.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ParseStats {
    pub total: u64,
    pub recognised: u64,
    pub structural_only: u64,
    pub skipped: u64,
}

impl ParseStats {
    pub fn record(&mut self, parsed: bool, structural: bool) {
        self.total += 1;
        if parsed {
            self.recognised += 1;
        } else if structural {
            self.structural_only += 1;
        } else {
            self.skipped += 1;
        }
    }

    pub fn coverage(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.recognised as f64 / self.total as f64
    }
}

/// Map the `MISSION_OBJECTIVE_STATE_<TOKEN>` suffix captured by
/// [`OBJECTIVE_UPSERTED_RE`] onto a typed state.
///
/// The catch-all is deliberate: an unrecognised token becomes `Unknown`
/// rather than being forced into the nearest known variant, so a state
/// CIG ships later is still counted (as resolved-but-not-completed)
/// instead of being silently discarded.
fn parse_objective_state(s: &str) -> MissionObjectiveState {
    match s {
        "INPROGRESS" => MissionObjectiveState::InProgress,
        "COMPLETED" => MissionObjectiveState::Completed,
        "FAILED" => MissionObjectiveState::Failed,
        "WITHDRAWN" => MissionObjectiveState::Withdrawn,
        _ => MissionObjectiveState::Unknown,
    }
}

/// Classify a structurally-parsed line into a typed `GameEvent`.
/// Returns `None` for lines we don't (yet) know how to route.
///
/// Two-layer dispatch:
///   1. If the line has an `<EventName>` shell, match on that first.
///   2. Otherwise (or on shell-match miss), fall through to
///      [`classify_body_prefix`] which scans the body for known
///      function-call-style entries (`SendShopBuyRequest`,
///      `CreateMissionObjectiveMarker`, ...). The engine emits some
///      events without a `<EventName>` shell, so a shell-only
///      dispatcher would silently drop them.
pub fn classify(line: &LogLine<'_>) -> Option<GameEvent> {
    let body = line.body;
    let ts = line.timestamp.to_string();

    let Some(event) = line.event_name else {
        return classify_body_prefix(&ts, body);
    };

    let shell_match: Option<GameEvent> = match event {
        "Init" if line.tags.contains(&"ProcessInit") => {
            let c = PROCESS_INIT_RE.captures(body)?;
            Some(GameEvent::ProcessInit(ProcessInit {
                timestamp: ts.clone(),
                local_session: c["local"].trim().to_string(),
                env_session: c["env"].trim().to_string(),
                online: &c["online"] == "1",
            }))
        }
        "Legacy login response" => {
            let c = LEGACY_LOGIN_RE.captures(body)?;
            Some(GameEvent::LegacyLogin(LegacyLogin {
                timestamp: ts.clone(),
                handle: c["handle"].to_string(),
                server_time: c.name("time").map(|m| m.as_str().to_string()),
            }))
        }
        "Join PU" => {
            let c = JOIN_PU_RE.captures(body)?;
            Some(GameEvent::JoinPu(JoinPu {
                timestamp: ts.clone(),
                address: c["address"].to_string(),
                port: c["port"].parse().ok()?,
                shard: c["shard"].to_string(),
                location_id: c["loc"].to_string(),
            }))
        }
        "Change Server Start" | "Change Server End" => {
            let c = CHANGE_SERVER_RE.captures(body)?;
            let phase = if event == "Change Server Start" {
                ServerPhase::Start
            } else {
                ServerPhase::End
            };
            Some(GameEvent::ChangeServer(ChangeServer {
                timestamp: ts.clone(),
                phase,
                is_shard_persisted: &c["shard"] == "1",
                is_server: &c["server"] == "1",
                is_multiplayer: &c["mp"] == "1",
                is_online: c.name("online").map(|m| m.as_str() == "1"),
            }))
        }
        "Seed Solar System" | "Seed Solar System Success" => {
            let c = SEED_SS_RE.captures(body)?;
            Some(GameEvent::SeedSolarSystem(SeedSolarSystem {
                timestamp: ts.clone(),
                solar_system: c["ss"].to_string(),
                shard: c["shard"].to_string(),
                success: event == "Seed Solar System Success",
            }))
        }
        "ResolveSpawnLocation Location Not Found" => {
            let c = RESOLVE_SPAWN_RE.captures(body)?;
            Some(GameEvent::ResolveSpawn(ResolveSpawn {
                timestamp: ts.clone(),
                player_geid: c["geid"].to_string(),
                fallback: true,
            }))
        }
        "Actor Death" => {
            let c = ACTOR_DEATH_RE.captures(body)?;
            Some(GameEvent::ActorDeath(ActorDeath {
                timestamp: ts.clone(),
                victim: c["victim"].to_string(),
                victim_geid: c.name("vgeid").map(|m| m.as_str().to_string()),
                zone: c["zone"].to_string(),
                killer: c["killer"].to_string(),
                killer_geid: c.name("kgeid").map(|m| m.as_str().to_string()),
                weapon: c["weapon"].to_string(),
                damage_type: c["dmg"].to_string(),
            }))
        }
        "Vehicle Destruction" => {
            let c = VEHICLE_DESTRUCTION_RE.captures(body)?;
            Some(GameEvent::VehicleDestruction(VehicleDestruction {
                timestamp: ts.clone(),
                vehicle_class: c["vclass"].to_string(),
                vehicle_id: c.name("vid").map(|m| m.as_str().to_string()),
                destroy_level: c["level"].parse().ok()?,
                caused_by: c["cause"].to_string(),
                zone: c.name("zone").map(|m| m.as_str().to_string()),
            }))
        }
        "SHUDEvent_OnNotification" => {
            let c = HUD_NOTIFICATION_RE.captures(body)?;
            let text = c["text"].to_string();
            let id: u64 = c["id"].parse().ok()?;
            // Promote "Incapacitated:" notifications to a dedicated
            // PlayerIncapacitated event so callers can distinguish the
            // recoverable downed state from generic HUD banners.
            // PlayerDeath fires separately ~30s later if the
            // "Time to Death" timer expires.
            if text.starts_with("Incapacitated:") {
                return Some(GameEvent::PlayerIncapacitated(PlayerIncapacitated {
                    timestamp: ts.clone(),
                    queue_id: id,
                    zone: None,
                }));
            }
            // Treat the all-zero mission/objective GUID as absent to keep
            // the wire payload tidy.
            let mission = c
                .name("mission")
                .map(|m| m.as_str().to_string())
                .filter(|m| m != "00000000-0000-0000-0000-000000000000");
            let objective_id = c
                .name("objective")
                .map(|m| m.as_str().to_string())
                .filter(|s| !s.trim().is_empty())
                .filter(|s| s != "00000000-0000-0000-0000-000000000000");
            Some(GameEvent::HudNotification(HudNotification {
                timestamp: ts.clone(),
                text,
                notification_id: id,
                mission_id: mission,
                objective_id,
            }))
        }
        "Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]" => {
            // Modern player-death signal. Only matches the FIRST line
            // of the cleanup burst — the one that names the player's
            // body item (`body_*`). Equipment-cleanup lines from the
            // same burst share this event_name but the regex rejects
            // them, so they fall through to unknown_event_samples
            // (where they're harmless noise).
            let c = PLAYER_DEATH_RE.captures(body)?;
            Some(GameEvent::PlayerDeath(PlayerDeath {
                timestamp: ts.clone(),
                body_class: c["body_class"].to_string(),
                body_id: c["body_id"].to_string(),
                zone: None,
            }))
        }
        "RequestLocationInventory" => {
            let c = LOCATION_INVENTORY_RE.captures(body)?;
            Some(GameEvent::LocationInventoryRequested(
                LocationInventoryRequested {
                    timestamp: ts.clone(),
                    player: c["player"].to_string(),
                    location: c["location"].to_string(),
                },
            ))
        }
        // Modern kiosk purchase (shell-tagged). The legacy
        // `SendShopBuyRequest(...)` function-call form is still handled by
        // classify_body_prefix; this catches the current bracketed format
        // and captures the economy fields (shop_name + aUEC price).
        "CEntityComponentShoppingProvider::SendStandardItemBuyRequest"
        | "CEntityComponentShopUIProvider::SendShopBuyRequest" => {
            let c = SHOP_BUY_MODERN_RE.captures(body)?;
            Some(GameEvent::ShopBuyRequest(ShopBuyRequest {
                timestamp: ts.clone(),
                shop_id: c.name("shop").map(|m| m.as_str().to_string()),
                item_class: c.name("item").map(|m| m.as_str().to_string()),
                quantity: None,
                raw: body.trim().to_string(),
                price: c
                    .name("price")
                    .and_then(|m| m.as_str().split('.').next())
                    .and_then(|s| s.parse().ok()),
                shop_name: c.name("shopname").map(|m| m.as_str().to_string()),
            }))
        }
        "InvalidateAllTerrainCells" => {
            let c = PLANET_TERRAIN_RE.captures(body)?;
            Some(GameEvent::PlanetTerrainLoad(PlanetTerrainLoad {
                timestamp: ts.clone(),
                planet: c["planet"].to_string(),
            }))
        }
        "AttachmentReceived" => {
            let c = ATTACHMENT_RECEIVED_RE.captures(body)?;
            Some(GameEvent::AttachmentReceived(AttachmentReceived {
                timestamp: ts.clone(),
                player: c["player"].to_string(),
                item_class: c["class"].to_string(),
                item_id: c["id"].to_string(),
                status: c["status"].to_string(),
                port: c["port"].to_string(),
                elapsed_seconds: c["elapsed"].parse().ok()?,
            }))
        }
        "EquipItem" => {
            let c = EQUIP_ITEM_RE.captures(body)?;
            Some(GameEvent::ItemEquipChange(ItemEquipChange {
                timestamp: ts.clone(),
                action: EquipAction::Equip,
                item_class: c["class"].to_string(),
                port: Some(c["port"].to_string()),
                items_count: None,
            }))
        }
        "StoreItem" => {
            let c = STORE_ITEM_RE.captures(body)?;
            Some(GameEvent::ItemEquipChange(ItemEquipChange {
                timestamp: ts.clone(),
                action: EquipAction::Store,
                item_class: c["class"].to_string(),
                port: None,
                items_count: c["count"].parse().ok(),
            }))
        }
        "Player Selected Quantum Target - Local"
        | "Player Requested Fuel to Quantum Target - Server Routing" => {
            let c = QUANTUM_TARGET_RE.captures(body)?;
            // Strip a trailing comma from destinations like
            // `OOC_Stanton_2_Crusader,` (the engine sometimes puts
            // a comma before "routing locally").
            let mut destination = c["dest"].to_string();
            if destination.ends_with(',') {
                destination.pop();
            }
            // Divert the mission/contract beacon form into the more
            // specific `MissionQuantumDestinationSelected`. This branch
            // lives inside the single shared dispatch arm, so the same
            // line can only ever yield ONE event — no duplicate with the
            // generic `QuantumTargetSelected`. Gated on the `Selected`
            // phase (the fuel-request precursor stays generic).
            if event == "Player Selected Quantum Target - Local" {
                if let Some(m) = MISSION_QT_BEACON_RE.captures(&destination) {
                    return Some(GameEvent::MissionQuantumDestinationSelected(
                        MissionQuantumDestinationSelected {
                            timestamp: ts.clone(),
                            beacon_id: m["beacon"].to_string(),
                            is_mission_destination: true,
                            travel_confirmed: false,
                        },
                    ));
                }
            }
            Some(GameEvent::QuantumTargetSelected(QuantumTargetSelected {
                timestamp: ts.clone(),
                phase: if event == "Player Selected Quantum Target - Local" {
                    QuantumTargetPhase::Selected
                } else {
                    QuantumTargetPhase::FuelRequested
                },
                vehicle_class: c["vehicle"].to_string(),
                vehicle_id: c["vid"].to_string(),
                destination,
            }))
        }
        "Quantum Drive Arrived - Arrived at Final Destination" => {
            let c = QUANTUM_ARRIVED_RE.captures(body)?;
            Some(GameEvent::QuantumArrived(QuantumArrived {
                timestamp: ts.clone(),
                vehicle_class: c["vehicle"].to_string(),
                vehicle_id: c["vid"].to_string(),
            }))
        }
        "Calculate Route" => {
            let c = CALCULATE_ROUTE_RE.captures(body)?;
            Some(GameEvent::QuantumRoute(QuantumRoute {
                timestamp: ts.clone(),
                start_system: c["start"].to_string(),
                destination: c["dest"].to_string(),
                vehicle_class: c["vehicle"].to_string(),
                vehicle_id: c["vid"].to_string(),
            }))
        }
        "CLocalMissionPhaseMarker::CreateMarker" => {
            let c = MISSION_PHASE_RE.captures(body)?;
            Some(GameEvent::MissionStart(MissionStart {
                timestamp: ts.clone(),
                mission_id: c["id"].to_string(),
                marker_kind: MissionMarkerKind::Phase,
                mission_name: c.name("name").map(|m| m.as_str().to_string()),
            }))
        }
        "EndMission" => {
            // EndMission body sometimes lacks both id AND outcome —
            // the regex's groups are all optional, so .captures() can
            // succeed with everything None. That's still a useful
            // signal: pair it with the most recent MissionStart.
            let c = MISSION_END_RE.captures(body);
            Some(GameEvent::MissionEnd(MissionEnd {
                timestamp: ts.clone(),
                mission_id: c
                    .as_ref()
                    .and_then(|c| c.name("id"))
                    .map(|m| m.as_str().to_string()),
                outcome: c
                    .as_ref()
                    .and_then(|c| c.name("outcome"))
                    .map(|m| m.as_str().to_string()),
            }))
        }
        "SystemQuit" => Some(GameEvent::SessionEnd(SessionEnd {
            timestamp: ts.clone(),
            kind: SessionEndKind::SystemQuit,
        })),
        "ObjectiveUpserted" => {
            let c = OBJECTIVE_UPSERTED_RE.captures(body)?;
            Some(GameEvent::MissionObjective(MissionObjective {
                timestamp: ts.clone(),
                objective_id: c["obj"].to_string(),
                mission_id: Some(c["mission"].to_string()),
                state: Some(parse_objective_state(&c["state"])),
                text: None,
            }))
        }
        "CMissionLogEntry::UpdateActiveObjective" => {
            let c = OBJECTIVE_LABEL_RE.captures(body)?;
            Some(GameEvent::MissionObjective(MissionObjective {
                timestamp: ts.clone(),
                objective_id: c["obj"].to_string(),
                mission_id: None,
                state: None,
                text: Some(c["text"].to_string()),
            }))
        }
        _ => None,
    };

    // Shell-match hit — return it. Otherwise fall through to body-prefix
    // dispatch: some lines DO have an event_name, but it's one we don't
    // recognise yet AND the body still contains a function-call entry
    // worth surfacing (rare, but cheap to handle).
    shell_match.or_else(|| classify_body_prefix(&ts, body))
}

/// Body-prefix dispatch for log lines whose meaningful payload sits in
/// the body itself rather than under an `<EventName>` shell. Star
/// Citizen logs a number of events as bare function calls
/// (`SendShopBuyRequest(...)`, `CreateMissionObjectiveMarker(...)`,
/// `CCIGBroker::FastShutdown(...)`); these never reach the shell-event
/// match arm and would otherwise be silently dropped.
///
/// The scan order matters: more specific patterns must come before
/// looser ones to avoid a shop-buy-request line being misclassified as
/// a generic shop response.
fn classify_body_prefix(ts: &str, body: &str) -> Option<GameEvent> {
    // Vehicle stow. Dispatched on the body marker rather than the shell
    // tag DELIBERATELY: this arrived under
    // `<CLandingArea::UnregisterFromExternalSystems>` until the
    // 2026-07-15-ish patch renamed it to
    // `<LandingArea_UnregisterFromExternalSystems_StowingVehicle>`,
    // whereupon a tag-keyed match stopped classifying and docking data
    // went silently empty for three weeks. CIG owns the tag string and
    // refactors it; `[STOWING ON UNREGISTER]` is the human-readable
    // marker they write into the body and it survived the rename
    // byte-for-byte. Keying on the body means the next rename is a
    // no-op for us. The marker is specific enough that no other line
    // shape carries it.
    if body.contains("[STOWING ON UNREGISTER]") {
        let c = VEHICLE_STOWED_RE.captures(body)?;
        let zone_host = c["zone_host"].to_string();
        // [0] is the sentinel for "no persistent zone" (PROC pads).
        let zone_host_id = if zone_host == "0" {
            None
        } else {
            Some(zone_host)
        };
        return Some(GameEvent::VehicleStowed(VehicleStowed {
            timestamp: ts.to_string(),
            vehicle_id: c["vehicle_id"].to_string(),
            landing_area: c["area"].to_string(),
            landing_area_id: c["area_id"].to_string(),
            zone_host_id,
        }));
    }

    // Mission objectives — function call inside body.
    if body.contains("CreateMissionObjectiveMarker") {
        let c = MISSION_OBJECTIVE_RE.captures(body)?;
        return Some(GameEvent::MissionStart(MissionStart {
            timestamp: ts.to_string(),
            mission_id: c["id"].to_string(),
            marker_kind: MissionMarkerKind::Objective,
            mission_name: None,
        }));
    }

    // Shop buy must be checked BEFORE shop response — both contain
    // "Shop", but only the buy form contains "BuyRequest".
    if body.contains("BuyRequest")
        && (body.contains("SendShopBuyRequest")
            || body.contains("SendStandardItemBuyRequest")
            || body.contains("SendStandardShopBuyRequest"))
    {
        let c = SHOP_BUY_RE.captures(body);
        return Some(GameEvent::ShopBuyRequest(ShopBuyRequest {
            timestamp: ts.to_string(),
            shop_id: c
                .as_ref()
                .and_then(|c| c.name("shop"))
                .map(|m| m.as_str().to_string()),
            item_class: c
                .as_ref()
                .and_then(|c| c.name("item"))
                .map(|m| m.as_str().to_string()),
            quantity: c
                .as_ref()
                .and_then(|c| c.name("qty"))
                .and_then(|m| m.as_str().parse().ok()),
            raw: body.trim().to_string(),
            price: None,
            shop_name: None,
        }));
    }

    if body.contains("ShopFlowResponse") {
        let c = SHOP_RESPONSE_RE.captures(body);
        let success = c
            .as_ref()
            .and_then(|c| c.name("result"))
            .map(|m| matches!(m.as_str(), "Success" | "OK" | "ok"));
        return Some(GameEvent::ShopFlowResponse(ShopFlowResponse {
            timestamp: ts.to_string(),
            shop_id: c
                .as_ref()
                .and_then(|c| c.name("shop"))
                .map(|m| m.as_str().to_string()),
            success,
            raw: body.trim().to_string(),
        }));
    }

    if body.contains("SendCommodityBuyRequest") {
        let c = COMMODITY_BUY_RE.captures(body);
        return Some(GameEvent::CommodityBuyRequest(CommodityBuyRequest {
            timestamp: ts.to_string(),
            commodity: c
                .as_ref()
                .and_then(|c| c.name("commodity"))
                .map(|m| m.as_str().to_string()),
            quantity: c
                .as_ref()
                .and_then(|c| c.name("qty"))
                .and_then(|m| m.as_str().parse().ok()),
            raw: body.trim().to_string(),
        }));
    }

    if body.contains("SendCommoditySellRequest") {
        let c = COMMODITY_SELL_RE.captures(body);
        return Some(GameEvent::CommoditySellRequest(CommoditySellRequest {
            timestamp: ts.to_string(),
            commodity: c
                .as_ref()
                .and_then(|c| c.name("commodity"))
                .map(|m| m.as_str().to_string()),
            quantity: c
                .as_ref()
                .and_then(|c| c.name("qty"))
                .and_then(|m| m.as_str().parse().ok()),
            raw: body.trim().to_string(),
        }));
    }

    if body.contains("CCIGBroker::FastShutdown") {
        return Some(GameEvent::SessionEnd(SessionEnd {
            timestamp: ts.to_string(),
            kind: SessionEndKind::FastShutdown,
        }));
    }

    None
}

/// Classify a structurally-parsed line and stamp default Observed
/// metadata in one call.
///
/// Returns `(event, metadata)`. Metadata is `Some(..)` exactly when
/// the line classified — so the two `Option`s share the same
/// presence. Inference passes that synthesise events or override
/// `source` / `confidence` mutate the returned metadata; this helper
/// only seeds the observed-default values.
pub fn classify_with_metadata(
    line: &LogLine<'_>,
    claimed_handle: Option<&str>,
) -> (Option<GameEvent>, Option<crate::metadata::EventMetadata>) {
    let event = classify(line);
    let meta = event
        .as_ref()
        .map(|e| crate::metadata::stamp(e, claimed_handle));
    (event, meta)
}

/// Outcome of [`classify_or_capture`]. Variants are exclusive: every
/// line resolves to exactly one of {classified, remote-matched,
/// unknown}. Callers that only care about the event payload can match
/// on the first two and ignore `Unknown`; callers that want to grow
/// rule coverage hold onto the `Unknown` for the user review queue.
#[derive(Debug)]
pub enum ClassifyOutcome {
    /// Built-in classifier matched.
    Classified(GameEvent),
    /// A remote parser rule matched.
    RemoteMatched(GameEvent),
    /// Neither matched — line is captured for review.
    Unknown(crate::unknown_lines::UnknownLine),
}

/// One-stop entry point: classify the line with both built-in and
/// remote rules; if neither matches, capture it as an unknown line for
/// the local review queue.
///
/// Built-ins are tried first so a remote rule can never override a
/// shipped classifier — see the note on [`apply_remote_rules`].
pub fn classify_or_capture(
    line: &LogLine<'_>,
    remote_rules: &[crate::parser_defs::CompiledRemoteRule],
    capture_ctx: &crate::unknown_lines::CaptureContextOwned,
    raw_line: &str,
    now_rfc3339: &str,
) -> ClassifyOutcome {
    if let Some(event) = classify(line) {
        return ClassifyOutcome::Classified(event);
    }
    if let Some(event) = crate::parser_defs::apply_remote_rules(line, remote_rules) {
        return ClassifyOutcome::RemoteMatched(event);
    }
    let captured =
        crate::unknown_lines::capture(raw_line, line.event_name, capture_ctx, now_rfc3339);
    ClassifyOutcome::Unknown(captured)
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn structural_parses_typical_notice_line() {
        let line = "<2026-05-02T21:14:23.189Z> [Notice] <Join PU> address[35.241.202.174] port[64300] shard[pub_euw1b_11704877_090] locationId[562954248454145] [Team_GameServices][GIM][Matchmaking]";
        let p = structural_parse(line).unwrap();
        assert_eq!(p.timestamp, "2026-05-02T21:14:23.189Z");
        assert_eq!(p.level, Some("Notice"));
        assert_eq!(p.event_name, Some("Join PU"));
        assert_eq!(p.tags, vec!["Team_GameServices", "GIM", "Matchmaking"]);
        assert!(p.body.contains("address[35.241.202.174]"));
    }

    #[test]
    fn structural_parses_line_without_event_or_tags() {
        let line = "<2026-05-02T21:01:51.390Z> Log started on Sat May  2 21:01:51 2026";
        let p = structural_parse(line).unwrap();
        assert_eq!(p.timestamp, "2026-05-02T21:01:51.390Z");
        assert_eq!(p.event_name, None);
        assert!(p.tags.is_empty());
    }

    #[test]
    fn structural_captures_nested_angle_event_name() {
        // CIG embeds lambda symbols like `<lambda_1>` inside event
        // names. The parser must capture the whole symbol, not stop
        // at the first inner `>`.
        let line = "<2026-05-03T18:00:00.000Z> [Notice] <CSCLoadingPlatformManager::LoadEntitiesReference::<lambda_1>> body here [Team_X]";
        let p = structural_parse(line).unwrap();
        assert_eq!(
            p.event_name,
            Some("CSCLoadingPlatformManager::LoadEntitiesReference::<lambda_1>")
        );
    }

    #[test]
    fn structural_returns_none_on_garbage() {
        assert_eq!(structural_parse("not a log line"), None);
        assert_eq!(structural_parse(""), None);
    }

    #[test]
    fn classifies_join_pu() {
        let line = "<2026-05-02T21:14:23.189Z> [Notice] <Join PU> address[35.241.202.174] port[64300] shard[pub_euw1b_11704877_090] locationId[562954248454145] [Team_GameServices][GIM][Matchmaking]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::JoinPu(j) => {
                assert_eq!(j.address, "35.241.202.174");
                assert_eq!(j.port, 64300);
                assert_eq!(j.shard, "pub_euw1b_11704877_090");
                assert_eq!(j.location_id, "562954248454145");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_legacy_login_response() {
        let line = "<2026-05-02T21:02:06.340Z> [Notice] <Legacy login response> [CIG-net] User Login Success - Handle[TheCodeSaiyan] - Time[213000609] [Team_GameServices][Login]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::LegacyLogin(l) => {
                assert_eq!(l.handle, "TheCodeSaiyan");
                assert_eq!(l.server_time.as_deref(), Some("213000609"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_change_server_start() {
        let line = "<2026-05-02T21:14:23.400Z> [Notice] <Change Server Start> IsShardPersisted[0] IsServer[1] IsMultiplayer[1] [Team_Network][Network][Loading]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::ChangeServer(c) => {
                assert_eq!(c.phase, ServerPhase::Start);
                assert!(!c.is_shard_persisted);
                assert!(c.is_server);
                assert!(c.is_multiplayer);
                assert!(c.is_online.is_none());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_resolve_spawn_with_player_geid() {
        let line = "<2026-05-02T21:02:10.956Z> [Notice] <ResolveSpawnLocation Location Not Found> Could not resolve initial spawn location from spawning module for player id: [9794883988961], setting spawn zone location zonehost to solar system fallback [Team_BackendServices][Services]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::ResolveSpawn(r) => {
                assert_eq!(r.player_geid, "9794883988961");
                assert!(r.fallback);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_hud_notification_jurisdiction() {
        let line = r#"<2026-05-03T17:57:19.579Z> [Notice] <SHUDEvent_OnNotification> Added notification "Entered Hurston Dynamics Jurisdiction: " [2] to queue. New queue size: 2, MissionId: [00000000-0000-0000-0000-000000000000], ObjectiveId: [] [Team_CoreGameplayFeatures][Missions][Comms]"#;
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::HudNotification(h) => {
                assert_eq!(h.text, "Entered Hurston Dynamics Jurisdiction: ");
                assert_eq!(h.notification_id, 2);
                // Zero-GUID is filtered out.
                assert!(h.mission_id.is_none());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_hud_notification_armistice() {
        let line = r#"<2026-05-03T17:58:32.955Z> [Notice] <SHUDEvent_OnNotification> Added notification "Entering Armistice Zone - Combat Prohibited: " [3] to queue. New queue size: 1, MissionId: [00000000-0000-0000-0000-000000000000], ObjectiveId: [] [Team_CoreGameplayFeatures][Missions][Comms]"#;
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        if let GameEvent::HudNotification(h) = event {
            assert!(h.text.contains("Armistice Zone"));
            assert_eq!(h.notification_id, 3);
        } else {
            panic!("expected HudNotification");
        }
    }

    #[test]
    fn hud_notification_captures_objective_id() {
        // A STITCHED Objective Complete record (Task 1 rejoins the halves).
        let line = "<2026-07-26T13:59:21.014Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Objective Complete: Go to a debris field above Euterpe: \" [15] to queue. New queue size: 1, MissionId: [7de35808-d909-4a6d-affe-edadf3e6fe77], ObjectiveId: [2432e890-93a3-0c46-a8a5-c7bb4915881f] [Team_CoreGameplayFeatures][Missions][Comms]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::HudNotification(h) => {
                assert_eq!(
                    h.text,
                    "Objective Complete: Go to a debris field above Euterpe: "
                );
                assert_eq!(h.notification_id, 15);
                assert_eq!(
                    h.mission_id.as_deref(),
                    Some("7de35808-d909-4a6d-affe-edadf3e6fe77")
                );
                assert_eq!(
                    h.objective_id.as_deref(),
                    Some("2432e890-93a3-0c46-a8a5-c7bb4915881f")
                );
            }
            other => panic!("expected HudNotification, got {other:?}"),
        }
    }

    #[test]
    fn hud_notification_empty_objective_id_is_none() {
        // Contract-level banners carry `ObjectiveId: []` — an empty bracket
        // must become None, not Some("").
        let line = "<2026-07-26T13:57:59.955Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Contract Accepted:  Combat Gauntlet - Scenario #5: \" [9] to queue. New queue size: 1, MissionId: [7de35808-d909-4a6d-affe-edadf3e6fe77], ObjectiveId: [] [Team_CoreGameplayFeatures][Missions][Comms]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::HudNotification(h) => {
                assert_eq!(h.objective_id, None);
                assert!(h.text.starts_with("Contract Accepted:"));
            }
            other => panic!("expected HudNotification, got {other:?}"),
        }
    }

    #[test]
    fn classifies_location_inventory_with_real_location() {
        let line = "<2026-05-03T18:09:28.551Z> [Notice] <RequestLocationInventory> Player[TheCodeSaiyan] requested inventory for Location[Stanton2_Orison] [Team_CoreGameplayFeatures][Inventory]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::LocationInventoryRequested(l) => {
                assert_eq!(l.player, "TheCodeSaiyan");
                assert_eq!(l.location, "Stanton2_Orison");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_location_inventory_invalid_passthrough() {
        // INVALID_LOCATION_ID is captured verbatim; downstream filters it.
        let line = "<2026-05-03T17:57:20.456Z> [Notice] <RequestLocationInventory> Player[TheCodeSaiyan] requested inventory for Location[INVALID_LOCATION_ID] [Team_CoreGameplayFeatures][Inventory]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        if let GameEvent::LocationInventoryRequested(l) = event {
            assert_eq!(l.location, "INVALID_LOCATION_ID");
        } else {
            panic!("expected LocationInventoryRequested");
        }
    }

    #[test]
    fn classifies_vehicle_stowed_hangar_elevator() {
        let line = "<2026-05-03T17:59:40.697Z> [Notice] <CLandingArea::UnregisterFromExternalSystems> [STOWING ON UNREGISTER] LandingArea_ShipElevator_HangarMediumTop [10012175369898] - Attempting to stow current vehicle [10006174073187] due to landing area unregistering. Vehicle Zone Host [10012175369590] [Team_MissionFeatures][ATC]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::VehicleStowed(v) => {
                assert_eq!(v.vehicle_id, "10006174073187");
                assert_eq!(v.landing_area, "LandingArea_ShipElevator_HangarMediumTop");
                assert_eq!(v.landing_area_id, "10012175369898");
                assert_eq!(v.zone_host_id.as_deref(), Some("10012175369590"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_vehicle_stowed_under_renamed_shell_tag() {
        // Real line captured from Game.log on 2026-07-30. Some time
        // around the 2026-07-15 patch CIG renamed the shell tag from
        // `CLandingArea::UnregisterFromExternalSystems` to
        // `LandingArea_UnregisterFromExternalSystems_StowingVehicle`
        // and appended `IsAuthorityPossible [N]` to the body. The old
        // classifier keyed on the tag, so every stow went unclassified
        // from 2026-07-14 onward — 3,307 lines in one local log set,
        // and the docking widget silently emptied. Stow is now
        // dispatched on the `[STOWING ON UNREGISTER]` body marker, so
        // the tag can be renamed again without breaking it.
        let line = "<2026-07-30T15:14:12.684Z> [Notice] <LandingArea_UnregisterFromExternalSystems_StowingVehicle> [STOWING ON UNREGISTER] LandingArea_ShipElevator_HangarSmallFront [745597122922] - Attempting to stow current vehicle [744589056229] due to landing area unregistering. Vehicle Zone Host [745597122719], IsAuthorityPossible [0] [Team_MissionFeatures][ATC]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::VehicleStowed(v) => {
                assert_eq!(v.vehicle_id, "744589056229");
                assert_eq!(v.landing_area, "LandingArea_ShipElevator_HangarSmallFront");
                assert_eq!(v.landing_area_id, "745597122922");
                assert_eq!(v.zone_host_id.as_deref(), Some("745597122719"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_vehicle_stowed_under_unknown_future_shell_tag() {
        // The body marker is the contract, not the tag. A tag we have
        // never seen must still classify — this is the assertion that
        // would have caught the July rename before it shipped.
        let line = "<2026-07-30T15:14:12.684Z> [Notice] <SomeTagCigHasNotInventedYet> [STOWING ON UNREGISTER] LandingArea_ShipElevator_HangarLargeTop [720809876838] - Attempting to stow current vehicle [742063885750] due to landing area unregistering. Vehicle Zone Host [720809876458], IsAuthorityPossible [0] [Team_MissionFeatures][ATC]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::VehicleStowed(v) => assert_eq!(v.vehicle_id, "742063885750"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_vehicle_stowed_proc_pad_drops_zero_zone() {
        // Procedurally-generated reststop pads emit `Vehicle Zone Host [0]`,
        // which our parser maps to `None` so consumers don't accidentally
        // treat the zero as a real GEID.
        let line = "<2026-05-03T18:03:45.145Z> [Notice] <CLandingArea::UnregisterFromExternalSystems> [STOWING ON UNREGISTER] [PROC]LandingArea_Pad_SmlB_{A27E3980-7BC8-42F5-A348-32E97E567C8B} [9917447339901] - Attempting to stow current vehicle [9945955166448] due to landing area unregistering. Vehicle Zone Host [0] [Team_MissionFeatures][ATC]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        if let GameEvent::VehicleStowed(v) = event {
            assert!(v.landing_area.starts_with("[PROC]LandingArea_Pad_SmlB_"));
            assert_eq!(v.vehicle_id, "9945955166448");
            assert!(v.zone_host_id.is_none());
        } else {
            panic!("expected VehicleStowed");
        }
    }

    #[test]
    fn classifies_attachment_received_armor() {
        let line = "<2026-05-03T17:52:57.219Z> [Notice] <AttachmentReceived> Player[TheCodeSaiyan] Attachment[rsi_odyssey_undersuit_01_01_01_200000000232, rsi_odyssey_undersuit_01_01_01, 200000000232] Status[persistent] Port[Armor_Undersuit] Elapsed[27.480394] [Team_CoreGameplayFeatures][Inventory]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::AttachmentReceived(a) => {
                assert_eq!(a.player, "TheCodeSaiyan");
                assert_eq!(a.item_class, "rsi_odyssey_undersuit_01_01_01");
                assert_eq!(a.item_id, "200000000232");
                assert_eq!(a.status, "persistent");
                assert_eq!(a.port, "Armor_Undersuit");
                assert!((a.elapsed_seconds - 27.480394).abs() < 1e-6);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_attachment_received_multitool_module() {
        let line = "<2026-05-03T19:19:05.518Z> [Notice] <AttachmentReceived> Player[TheCodeSaiyan] Attachment[grin_multitool_01_tractorbeam_9994535389060, grin_multitool_01_tractorbeam, 9994535389060] Status[persistent] Port[module_attach] Elapsed[1375.079224] [Team_CoreGameplayFeatures][Inventory]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        if let GameEvent::AttachmentReceived(a) = event {
            assert_eq!(a.item_class, "grin_multitool_01_tractorbeam");
            assert_eq!(a.port, "module_attach");
            assert!(a.elapsed_seconds > 1000.0);
        } else {
            panic!("expected AttachmentReceived");
        }
    }

    #[test]
    fn classifies_quantum_target_selected() {
        let line = "<2026-05-03T18:03:22.115Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][45668] | NOT AUTH | CRUS_Starfighter_Ion_10012175043656[10012175043656]|CSCItemNavigation::OnPlayerSelectedQuantumTarget|Player has selected point OOC_Stanton_2_Crusader as their destination, routing locally [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::QuantumTargetSelected(q) => {
                assert_eq!(q.phase, QuantumTargetPhase::Selected);
                assert_eq!(q.vehicle_class, "CRUS_Starfighter_Ion");
                assert_eq!(q.vehicle_id, "10012175043656");
                assert_eq!(q.destination, "OOC_Stanton_2_Crusader");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_quantum_target_fuel_requested() {
        let line = "<2026-05-03T18:07:46.738Z> [Notice] <Player Requested Fuel to Quantum Target - Server Routing> [ItemNavigation][CL][45668] | NOT AUTH | CRUS_Starfighter_Ion_10012175043656[10012175043656]|CSCItemNavigation::OnPlayerRequestFuelToQuantumTarget|Player has requested fuel calculation to destination Orison_LOC [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        if let GameEvent::QuantumTargetSelected(q) = event {
            assert_eq!(q.phase, QuantumTargetPhase::FuelRequested);
            assert_eq!(q.vehicle_class, "CRUS_Starfighter_Ion");
            assert_eq!(q.destination, "Orison_LOC");
        } else {
            panic!("expected QuantumTargetSelected");
        }
    }

    /// REAL corpus lines. Measured 2026-07-31: 1,090 of these sat
    /// unparsed while target-selection and route-calculation were both
    /// captured, so a quantum trip had a start and no end and completion
    /// could only be inferred from absence.
    #[test]
    fn classifies_quantum_arrived_from_real_lines() {
        for (line, class, id) in [
            ("<2026-07-06T11:46:40.223Z> [Notice] <Quantum Drive Arrived - Arrived at Final Destination> [ItemNavigation][CL][20612] | NOT AUTH | ARGO_MOLE_662886645202[662886645202]|CSCItemNavigation::OnQuantumDriveArrived|Quantum Drive has arrived at final destination [Team_CGP4][QuantumTravel]",
             "ARGO_MOLE", "662886645202"),
            ("<2026-07-18T23:58:12.313Z> [Notice] <Quantum Drive Arrived - Arrived at Final Destination> [ItemNavigation][CL][39180] | NOT AUTH | GAMA_Railen_723256764257[723256764257]|CSCItemNavigation::OnQuantumDriveArrived|Quantum Drive has arrived at final destination [Team_CGP4][QuantumTravel]",
             "GAMA_Railen", "723256764257"),
        ] {
            let p = structural_parse(line).unwrap();
            match classify(&p).unwrap() {
                GameEvent::QuantumArrived(a) => {
                    assert_eq!(a.vehicle_class, class);
                    assert_eq!(a.vehicle_id, id);
                }
                other => panic!("expected QuantumArrived, got {other:?}"),
            }
        }
    }

    #[test]
    fn quantum_arrival_is_anchored_on_the_arrival_method() {
        // A line under the same tag but a DIFFERENT navigation method
        // must not classify as a completed trip. Without the anchor a
        // future "arrived at waypoint" line would silently inflate the
        // completion count.
        let line = "<2026-07-06T11:46:40.223Z> [Notice] <Quantum Drive Arrived - Arrived at Final Destination> [ItemNavigation][CL][20612] | NOT AUTH | ARGO_MOLE_662886645202[662886645202]|CSCItemNavigation::OnSomethingElse|Quantum Drive has arrived somewhere [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        assert!(
            classify(&p).is_none(),
            "a different navigation method must not read as arrival"
        );
    }

    #[test]
    fn classifies_quantum_route() {
        let line = "<2026-07-16T22:33:03.269Z> [Notice] <Calculate Route> [ItemNavigation][CL][20876] | NOT AUTH | ANVL_Hornet_F7CS_Mk2_721505167979[721505167979]|CSCItemNavigation::CalculateRoute|Projected Start Location is Stanton for route to destination PartyMemberMarker_718290597259 [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::QuantumRoute(r) => {
                assert_eq!(r.vehicle_class, "ANVL_Hornet_F7CS_Mk2");
                assert_eq!(r.vehicle_id, "721505167979");
                assert_eq!(r.start_system, "Stanton");
                assert_eq!(r.destination, "PartyMemberMarker_718290597259");
            }
            other => panic!("expected QuantumRoute, got {other:?}"),
        }
    }

    // --- Mission / contract quantum-beacon destination selection (M9) ---

    /// Test 1 + 2 + 9: the verbatim example line classifies as the new
    /// `MissionQuantumDestinationSelected` variant, captures the beacon
    /// id, and carries the intent-only invariants.
    #[test]
    fn classifies_mission_quantum_destination_selected() {
        let line = "<2026-07-20T17:45:35.741Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][7948] | NOT AUTH | RSI_Perseus_726713445335[726713445335]|CSCItemNavigation::OnPlayerSelectedQuantumTarget|Player has selected point MISSION_QT_Quantum_Beacon_720518925602 as their destination, routing locally [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::MissionQuantumDestinationSelected(m) => {
                assert_eq!(m.beacon_id, "720518925602");
                assert!(m.is_mission_destination);
                assert!(!m.travel_confirmed, "selection is not travel start");
                assert_eq!(m.timestamp, "2026-07-20T17:45:35.741Z");
            }
            other => panic!("expected MissionQuantumDestinationSelected, got {other:?}"),
        }
    }

    /// Test 3: a completely different ship type (no `RSI_Perseus`) still
    /// matches — the ship class/id is not hardcoded.
    #[test]
    fn mission_qt_matches_different_ship_type() {
        let line = "<2026-07-20T18:00:00.000Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][12345] | NOT AUTH | AEGS_Gladius_800400400400[800400400400]|CSCItemNavigation::OnPlayerSelectedQuantumTarget|Player has selected point MISSION_QT_Quantum_Beacon_111222333 as their destination, routing locally [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::MissionQuantumDestinationSelected(m) => {
                assert_eq!(m.beacon_id, "111222333");
            }
            other => panic!("expected MissionQuantumDestinationSelected, got {other:?}"),
        }
    }

    /// Test 4: different thread / entity id / beacon id all still match.
    #[test]
    fn mission_qt_matches_different_thread_entity_beacon() {
        let line = "<2025-01-02T03:04:05.006Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][99999] | NOT AUTH | DRAK_Cutlass_Black_555000555000[555000555000]|CSCItemNavigation::OnPlayerSelectedQuantumTarget|Player has selected point MISSION_QT_Quantum_Beacon_424242 as their destination, routing locally [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::MissionQuantumDestinationSelected(m) => {
                assert_eq!(m.beacon_id, "424242");
            }
            other => panic!("expected MissionQuantumDestinationSelected, got {other:?}"),
        }
    }

    /// Test 5: an authed line (`| AUTH |` instead of `| NOT AUTH |`)
    /// still matches — the auth-state text is not part of the shape.
    #[test]
    fn mission_qt_matches_authed_line() {
        let line = "<2026-07-20T17:45:35.741Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][7948] | AUTH | RSI_Perseus_726713445335[726713445335]|CSCItemNavigation::OnPlayerSelectedQuantumTarget|Player has selected point MISSION_QT_Quantum_Beacon_720518925602 as their destination, routing locally [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::MissionQuantumDestinationSelected(m) => {
                assert_eq!(m.beacon_id, "720518925602");
            }
            other => panic!("expected MissionQuantumDestinationSelected, got {other:?}"),
        }
    }

    /// Test 6 + 15: a normal (non-mission) quantum destination does NOT
    /// produce the mission variant — it stays a generic
    /// `QuantumTargetSelected`. Because `classify` returns exactly one
    /// event per line, this same assertion proves the example line can
    /// never emit BOTH events (no duplicate from the overlapping shapes).
    #[test]
    fn normal_quantum_destination_is_not_mission_variant() {
        let line = "<2026-05-03T18:03:22.115Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][45668] | NOT AUTH | CRUS_Starfighter_Ion_10012175043656[10012175043656]|CSCItemNavigation::OnPlayerSelectedQuantumTarget|Player has selected point OOC_Stanton_2_Crusader as their destination, routing locally [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        assert!(
            matches!(event, GameEvent::QuantumTargetSelected(_)),
            "normal destination must stay generic, got {event:?}"
        );
        assert!(
            !matches!(event, GameEvent::MissionQuantumDestinationSelected(_)),
            "must not divert a non-mission destination into the mission variant"
        );
    }

    /// Test 15 (explicit): the example mission line yields exactly one
    /// classified event, and it is the mission variant — never a second
    /// `QuantumTargetSelected` for the same line.
    #[test]
    fn mission_qt_line_yields_single_event_no_duplicate() {
        let line = "<2026-07-20T17:45:35.741Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][7948] | NOT AUTH | RSI_Perseus_726713445335[726713445335]|CSCItemNavigation::OnPlayerSelectedQuantumTarget|Player has selected point MISSION_QT_Quantum_Beacon_720518925602 as their destination, routing locally [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        // `classify` returns a single `Option<GameEvent>`: one line → one
        // event. The mission branch lives inside the one shared dispatch
        // arm, so the generic `QuantumTargetSelected` path is structurally
        // unreachable for this line.
        assert!(matches!(
            event,
            GameEvent::MissionQuantumDestinationSelected(_)
        ));
    }

    /// Test 7: the mission beacon string under a DIFFERENT event type
    /// (`<Calculate Route>`) does NOT produce the mission variant — the
    /// detection is tied to the quantum-target-selected dispatch arm.
    #[test]
    fn mission_beacon_under_different_event_type_does_not_match() {
        let line = "<2026-07-20T17:46:00.000Z> [Notice] <Calculate Route> [ItemNavigation][CL][20876] | NOT AUTH | ANVL_Hornet_F7CS_Mk2_721505167979[721505167979]|CSCItemNavigation::CalculateRoute|Projected Start Location is Stanton for route to destination MISSION_QT_Quantum_Beacon_720518925602 [Team_CGP4][QuantumTravel]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        assert!(
            matches!(event, GameEvent::QuantumRoute(_)),
            "a beacon under <Calculate Route> must classify as QuantumRoute, got {event:?}"
        );
        assert!(!matches!(
            event,
            GameEvent::MissionQuantumDestinationSelected(_)
        ));
    }

    /// Test 8: malformed / truncated lines do NOT produce the mission
    /// variant. A line truncated before the routing body fails to
    /// classify at all; a non-numeric beacon token falls through to the
    /// generic variant rather than the mission one.
    #[test]
    fn malformed_mission_qt_lines_do_not_match() {
        // Truncated — no `| … |Player has selected point …` body.
        let truncated = "<2026-07-20T17:45:35.741Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][7948]";
        let p = structural_parse(truncated).unwrap();
        assert!(
            classify(&p).is_none(),
            "a truncated selection line must not classify"
        );

        // Non-numeric beacon id — not the exact `MISSION_QT_Quantum_Beacon_<digits>`
        // form, so it stays a generic QuantumTargetSelected.
        let non_numeric = "<2026-07-20T17:45:35.741Z> [Notice] <Player Selected Quantum Target - Local> [ItemNavigation][CL][7948] | NOT AUTH | RSI_Perseus_726713445335[726713445335]|CSCItemNavigation::OnPlayerSelectedQuantumTarget|Player has selected point MISSION_QT_Quantum_Beacon_notanumber as their destination, routing locally [Team_CGP4][QuantumTravel]";
        let p = structural_parse(non_numeric).unwrap();
        let event = classify(&p).unwrap();
        assert!(
            !matches!(event, GameEvent::MissionQuantumDestinationSelected(_)),
            "non-numeric beacon must not become the mission variant, got {event:?}"
        );
    }

    #[test]
    fn classifies_item_equip() {
        let line = "<2026-07-17T03:03:52.007Z> [Notice] <EquipItem> Request[34] equip from Inventory[204056438813:Location:2147648880] Class[crlf_consumable_healing_01] Rank[amqqewapqjuxaacy] Port[Body_ItemPort:Armor_Undersuit:oxyPen_attach_1] DependentRequest[4294967295] PostAction[None] [Team_CoreGameplayFeatures][Inventory]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::ItemEquipChange(e) => {
                assert_eq!(e.action, EquipAction::Equip);
                assert_eq!(e.item_class, "crlf_consumable_healing_01");
                assert_eq!(
                    e.port.as_deref(),
                    Some("Body_ItemPort:Armor_Undersuit:oxyPen_attach_1")
                );
                assert_eq!(e.items_count, None);
            }
            other => panic!("expected ItemEquipChange, got {other:?}"),
        }
    }

    #[test]
    fn classifies_item_store() {
        let line = "<2026-07-17T03:03:15.948Z> [Notice] <StoreItem> Request[9] store 'klwe_rifle_energy_01_gold01_721587543491' [721587543491] by 'TheCodeSaiyan' [204056438813] To Inventory[INVALID] Class[klwe_rifle_energy_01_gold01] Rank[amqqewctvlwjn] ItemsCount[0] [Team_CoreGameplayFeatures][Inventory]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::ItemEquipChange(e) => {
                assert_eq!(e.action, EquipAction::Store);
                assert_eq!(e.item_class, "klwe_rifle_energy_01_gold01");
                assert_eq!(e.items_count, Some(0));
                assert_eq!(e.port, None);
            }
            other => panic!("expected ItemEquipChange, got {other:?}"),
        }
    }

    #[test]
    fn classifies_mission_objective_upserted() {
        let line = "<2026-07-16T19:34:39.774Z> [Notice] <ObjectiveUpserted> Received ObjectiveUpserted push message for: mission_id cb056802-2432-4f61-8e5c-aa38c4920308 - objective_id 964eae97-d16c-7b39-c8cc-220e349596b4 - state MISSION_OBJECTIVE_STATE_INPROGRESS - created 0 - flags=ShowInLog|ShowProgress|RespectInheritedVisibility| [Team_GameServices][Missions]";
        let p = structural_parse(line).unwrap();
        let ev = classify(&p).unwrap();
        match ev {
            GameEvent::MissionObjective(m) => {
                assert_eq!(m.objective_id, "964eae97-d16c-7b39-c8cc-220e349596b4");
                assert_eq!(
                    m.mission_id.as_deref(),
                    Some("cb056802-2432-4f61-8e5c-aa38c4920308")
                );
                assert_eq!(m.state, Some(MissionObjectiveState::InProgress));
                assert_eq!(m.text, None);
            }
            other => panic!("expected MissionObjective, got {other:?}"),
        }
    }

    #[test]
    fn classifies_mission_objective_completed_state() {
        let line = "<2026-07-16T19:34:39.774Z> [Notice] <ObjectiveUpserted> Received ObjectiveUpserted push message for: mission_id cb056802-2432-4f61-8e5c-aa38c4920308 - objective_id 964eae97-d16c-7b39-c8cc-220e349596b4 - state MISSION_OBJECTIVE_STATE_COMPLETED - created 0 - flags=ShowInLog| [Team_GameServices][Missions]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::MissionObjective(m) => {
                assert_eq!(m.state, Some(MissionObjectiveState::Completed))
            }
            other => panic!("expected MissionObjective, got {other:?}"),
        }
    }

    #[test]
    fn classifies_mission_objective_withdrawn_state() {
        // WITHDRAWN is a real terminal state, not an unrecognised one: the
        // objective RESOLVED and was not completed. Asserted on the
        // SERIALISED string rather than the enum because that string is
        // what lands in `events.payload->>'state'` and what the
        // `objective_outcomes` rank tables match on -- an enum-only
        // assertion would sail past a serde rename and leave the
        // aggregate silently dropping the row.
        let line = "<2026-07-16T19:41:02.118Z> [Notice] <ObjectiveUpserted> Received ObjectiveUpserted push message for: mission_id cb056802-2432-4f61-8e5c-aa38c4920308 - objective_id 964eae97-d16c-7b39-c8cc-220e349596b4 - state MISSION_OBJECTIVE_STATE_WITHDRAWN - created 0 - flags=ShowInLog| [Team_GameServices][Missions]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::MissionObjective(m) => {
                assert_eq!(
                    serde_json::to_value(m.state).unwrap(),
                    serde_json::json!("withdrawn"),
                );
            }
            other => panic!("expected MissionObjective, got {other:?}"),
        }
    }

    #[test]
    fn classifies_mission_objective_unrecognised_state_stays_unknown() {
        // The catch-all must survive the WITHDRAWN mapping: a state token
        // CIG has not shipped yet still parses to `unknown` rather than
        // being forced into the nearest known variant.
        let line = "<2026-07-16T19:41:02.118Z> [Notice] <ObjectiveUpserted> Received ObjectiveUpserted push message for: mission_id cb056802-2432-4f61-8e5c-aa38c4920308 - objective_id 964eae97-d16c-7b39-c8cc-220e349596b4 - state MISSION_OBJECTIVE_STATE_SOMETHINGNEW - created 0 - flags=ShowInLog| [Team_GameServices][Missions]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::MissionObjective(m) => {
                assert_eq!(m.state, Some(MissionObjectiveState::Unknown));
            }
            other => panic!("expected MissionObjective, got {other:?}"),
        }
    }

    #[test]
    fn classifies_mission_objective_label_keeps_template_text_verbatim() {
        // Text is often an unexpanded template -- store verbatim, never rewrite.
        let line = "<2026-07-16T19:34:38.265Z> [Notice] <CMissionLogEntry::UpdateActiveObjective> Objective updated id=964eae97-d16c-7b39-c8cc-220e349596b4, flags=ShowInLog|, hidden=0, hiddenInUI=0, markerHidden=0, uiDisplay[Priority=1][Text=Go to ~mission(Location)] [Team_MissionFeatures][Missions]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::MissionObjective(m) => {
                assert_eq!(m.objective_id, "964eae97-d16c-7b39-c8cc-220e349596b4");
                assert_eq!(m.text.as_deref(), Some("Go to ~mission(Location)"));
                assert_eq!(m.mission_id, None);
                assert_eq!(m.state, None);
            }
            other => panic!("expected MissionObjective, got {other:?}"),
        }
    }

    #[test]
    fn classifies_planet_terrain_load() {
        let line = "<2026-05-03T18:00:00.000Z> [Notice] <InvalidateAllTerrainCells> Planet OOC_Stanton_2b_Daymar invalidated all terrain cells [Team_3DEngine]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::PlanetTerrainLoad(t) => {
                assert_eq!(t.planet, "OOC_Stanton_2b_Daymar");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_synthetic_actor_death() {
        // Synthesised — this fixture has no combat.
        let line = "<2024-01-15T14:30:25.832Z> [Notice] <Actor Death> CActor::Kill: 'VictimName' [123456789] in zone 'OOC_Stanton_4a_PortOlisar' killed by 'KillerName' [987654321] using 'Weapon_Pistol_Behring_P4AR_Default' [Class P4AR] with damage type 'Bullet' from direction x: 0.5, y: 0.2, z: -0.8 [Team_ActorTech][Actor]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::ActorDeath(d) => {
                assert_eq!(d.victim, "VictimName");
                assert_eq!(d.killer, "KillerName");
                assert_eq!(d.zone, "OOC_Stanton_4a_PortOlisar");
                assert_eq!(d.weapon, "Weapon_Pistol_Behring_P4AR_Default");
                assert_eq!(d.damage_type, "Bullet");
                assert_eq!(d.victim_geid.as_deref(), Some("123456789"));
                assert_eq!(d.killer_geid.as_deref(), Some("987654321"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_modern_player_death_from_real_capture() {
        // Real line from the user's logbackups directory, redacted of
        // secrets (none in this line). Modern (4.x+) SC writes player
        // deaths as a corpse-cleanup burst; the FIRST line, naming the
        // body item, is the death signal.
        let line = "<2026-05-01T18:46:15.085Z> [Notice] <Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]> Item 'body_01_noMagicPocket_9754924365641 - Class(body_01_noMagicPocket) - Context(Streamable Runtime-spawned) - Socpak()', Recorded data is: Port Name 'Body_ItemPort', Class GUID: 'dbaa8a7d-755f-4104-8b24-7b58fd1e76f6', KeptId: '9754924365641' [Team_CoreGameplayFeatures][Unknown]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::PlayerDeath(d) => {
                assert_eq!(d.timestamp, "2026-05-01T18:46:15.085Z");
                assert_eq!(d.body_class, "body_01_noMagicPocket");
                assert_eq!(d.body_id, "9754924365641");
                assert_eq!(d.zone, None);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn modern_death_burst_only_classifies_the_body_line() {
        // The same burst spawns equipment-cleanup lines that share
        // the event_name but don't start with `body_`. They MUST NOT
        // classify as PlayerDeath — otherwise one death would surface
        // as a dozen events in the timeline.
        let undersuit = "<2026-05-01T18:46:15.087Z> [Notice] <Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]> Item 'vgl_undersuit_01_01_13_9982571228598 - Class(vgl_undersuit_01_01_13) - Context(Streamable Runtime-spawned) - Socpak()', Recorded data is: Port Name 'Armor_Undersuit', Class GUID: '1ae7202d-b9c0-4492-87e6-a46b2e80fc56' [Team_CoreGameplayFeatures][Unknown]";
        let p = structural_parse(undersuit).unwrap();
        // classify returns None — the body-anchored regex rejects
        // non-body items in the burst.
        assert!(classify(&p).is_none());
    }

    #[test]
    fn classifies_player_incapacitated_from_real_capture() {
        // Real "Incapacitated" notification line from the user's logs.
        // Goes through SHUDEvent_OnNotification but the body-text
        // discriminator promotes it to PlayerIncapacitated.
        let line = "<2026-05-01T18:45:45.141Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Incapacitated: While incapacitated, ask others in your party, in chat, or through rescue service beacons to revive you before the 'Time to Death' timer expires.\" [89] to queue. New queue size: 1, MissionId: [00000000-0000-0000-0000-000000000000], ObjectiveId: [] [Team_CoreGameplayFeatures][Missions][Comms]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::PlayerIncapacitated(i) => {
                assert_eq!(i.timestamp, "2026-05-01T18:45:45.141Z");
                assert_eq!(i.queue_id, 89);
                assert_eq!(i.zone, None);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn non_incap_hud_notification_still_routes_to_hud_notification() {
        // Sanity: the dedicated PlayerIncapacitated branch must NOT
        // hijack other HUD banners. Generic "Entered jurisdiction"
        // text stays as HudNotification.
        let line = "<2026-05-02T12:00:00.000Z> [Notice] <SHUDEvent_OnNotification> Added notification \"Entered Hurston Dynamics Jurisdiction: \" [42] to queue. New queue size: 1, MissionId: [00000000-0000-0000-0000-000000000000], ObjectiveId: [] [Team][UI]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        assert!(matches!(event, GameEvent::HudNotification(_)));
    }

    // -- Launcher log parsing -------------------------------------

    #[test]
    fn parse_launcher_line_extracts_timestamp_level_and_message() {
        let line = "[2026-05-06 12:34:56.789] [info] Starting RSI Launcher 2.10.4";
        let p = parse_launcher_line(line).unwrap();
        assert_eq!(p.timestamp, "2026-05-06 12:34:56.789");
        assert_eq!(p.level, "info");
        assert_eq!(p.message, "Starting RSI Launcher 2.10.4");
    }

    #[test]
    fn parse_launcher_line_handles_warn_and_error_levels() {
        let warn =
            parse_launcher_line("[2026-05-06 12:00:00.000] [warn] Patcher retrying").unwrap();
        assert_eq!(warn.level, "warn");
        let err =
            parse_launcher_line("[2026-05-06 12:00:01.000] [error] Failed to authenticate user")
                .unwrap();
        assert_eq!(err.level, "error");
        assert_eq!(err.message, "Failed to authenticate user");
    }

    #[test]
    fn parse_launcher_line_returns_none_for_blank_or_continuation_lines() {
        assert!(parse_launcher_line("").is_none());
        // Lines without the bracket shell don't parse — these are
        // typically multi-line continuations of a previous entry.
        assert!(parse_launcher_line("    at Object.<anonymous> (file.js:10)").is_none());
        assert!(parse_launcher_line("[just-a-banner]").is_none());
    }

    #[test]
    fn parse_launcher_line_drops_empty_message_bodies() {
        // The launcher sometimes emits a header line with no body —
        // we don't surface those because they carry no signal.
        assert!(parse_launcher_line("[2026-05-06 12:00:00.000] [info] ").is_none());
    }

    #[test]
    fn parse_launcher_line_preserves_message_with_brackets() {
        // Message bodies often contain bracketed sub-fields (URLs,
        // module names). Make sure we don't truncate at the first
        // post-prefix `]`.
        let line =
            "[2026-05-06 12:00:00.000] [info] GET https://example.com/[v1]/assets returned 200";
        let p = parse_launcher_line(line).unwrap();
        assert_eq!(
            p.message,
            "GET https://example.com/[v1]/assets returned 200"
        );
    }

    // -- Launcher message classification ---------------------------

    #[test]
    fn classify_launcher_message_routes_error_level_to_error_bucket() {
        assert_eq!(
            classify_launcher_message("error", "Something went sideways"),
            LauncherCategory::Error
        );
        assert_eq!(
            classify_launcher_message("fatal", "Engine died"),
            LauncherCategory::Error
        );
    }

    #[test]
    fn classify_launcher_message_routes_failure_keywords_at_info_level_to_error() {
        // The launcher logs some failures at info level. Body keywords
        // win over the level — these should still be `Error`.
        assert_eq!(
            classify_launcher_message("info", "Patch download failed"),
            LauncherCategory::Error
        );
        assert_eq!(
            classify_launcher_message("info", "ERR_NETWORK_TIMEOUT"),
            LauncherCategory::Error
        );
    }

    #[test]
    fn classify_launcher_message_recognises_auth_phrases() {
        for msg in [
            "User logged in successfully",
            "Logging in with token",
            "Session refresh in progress",
            "OAuth callback received",
            "Saved credential for user",
            "User signed in",
        ] {
            assert_eq!(
                classify_launcher_message("info", msg),
                LauncherCategory::Auth,
                "did not classify as Auth: {msg}"
            );
        }
    }

    #[test]
    fn classify_launcher_message_recognises_install_and_patch_separately() {
        assert_eq!(
            classify_launcher_message("info", "Installing Star Citizen LIVE"),
            LauncherCategory::Install
        );
        assert_eq!(
            classify_launcher_message("info", "Verifying file integrity"),
            LauncherCategory::Install
        );
        assert_eq!(
            classify_launcher_message("info", "Applying patch 4.0"),
            LauncherCategory::Patch
        );
        assert_eq!(
            classify_launcher_message("info", "Patch download complete"),
            LauncherCategory::Patch
        );
    }

    #[test]
    fn classify_launcher_message_recognises_update_keywords() {
        assert_eq!(
            classify_launcher_message("info", "Checking for updates"),
            LauncherCategory::Update
        );
        assert_eq!(
            classify_launcher_message("info", "Self-update available"),
            LauncherCategory::Update
        );
    }

    #[test]
    fn classify_launcher_message_falls_back_to_info_when_unknown() {
        assert_eq!(
            classify_launcher_message("info", "Something quiet happened"),
            LauncherCategory::Info
        );
        assert_eq!(
            classify_launcher_message("debug", "Heartbeat tick"),
            LauncherCategory::Info
        );
    }

    // -----------------------------------------------------------------
    // v0.2.0-alpha — mission / shop / commodity / session-end
    //
    // Synthetic fixtures only — these exercise the regex shapes, not
    // real captures. Once we have real captures in hand, replace
    // these with verbatim log lines and tighten the patterns.
    // -----------------------------------------------------------------

    #[test]
    fn classifies_mission_phase_marker() {
        let line = "<2026-05-07T12:00:00.000Z> [Notice] <CLocalMissionPhaseMarker::CreateMarker> missionId[ABC12345-0000-0000-0000-000000000001] missionName[Hauling Run] [Team_Missions]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::MissionStart(m) => {
                assert_eq!(m.mission_id, "ABC12345-0000-0000-0000-000000000001");
                assert_eq!(m.marker_kind, MissionMarkerKind::Phase);
                assert_eq!(m.mission_name.as_deref(), Some("Hauling Run"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_mission_objective_marker_via_body_prefix() {
        // No <EventName> shell — the meaningful payload is a function
        // call sitting in the body. Must reach via body-prefix dispatch.
        let line = "<2026-05-07T12:01:00.000Z> [Notice] CreateMissionObjectiveMarker(11111111-2222-3333-4444-555555555555, deliver_to_lorville)";
        let p = structural_parse(line).unwrap();
        assert_eq!(p.event_name, None, "fixture must lack a shell event");
        let event = classify(&p).unwrap();
        match event {
            GameEvent::MissionStart(m) => {
                assert_eq!(m.mission_id, "11111111-2222-3333-4444-555555555555");
                assert_eq!(m.marker_kind, MissionMarkerKind::Objective);
                assert!(m.mission_name.is_none());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_end_mission_with_outcome() {
        let line = "<2026-05-07T12:30:00.000Z> [Notice] <EndMission> missionId[ABC12345-0000-0000-0000-000000000001] outcome[Success] [Team_Missions]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::MissionEnd(m) => {
                assert_eq!(
                    m.mission_id.as_deref(),
                    Some("ABC12345-0000-0000-0000-000000000001")
                );
                assert_eq!(m.outcome.as_deref(), Some("Success"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_shop_buy_request_via_body_prefix() {
        let line = "<2026-05-07T13:00:00.000Z> [Notice] SendShopBuyRequest(shopId=area18_kiosk_07, itemClass=apparel_helmet_pilot_blue, quantity=1)";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::ShopBuyRequest(s) => {
                assert_eq!(s.shop_id.as_deref(), Some("area18_kiosk_07"));
                assert_eq!(s.item_class.as_deref(), Some("apparel_helmet_pilot_blue"));
                assert_eq!(s.quantity, Some(1));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_modern_kiosk_shop_buy_with_shop_and_price() {
        // Modern shell-tagged kiosk buy line: captures shop_name + aUEC price.
        let line = "<2026-07-16T20:35:21.276Z> [Notice] <CEntityComponentShoppingProvider::SendStandardItemBuyRequest> Sending SShopBuyRequest - playerId[204056438813] shopId[719523767693] shopName[SCShop_Aparelli_NewBabbage] kioskId[0] client_price[2200.000000] itemClassGUID[33dabc-guid] [Team_CoreGameplayFeatures][Shopping]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::ShopBuyRequest(s) => {
                assert_eq!(s.shop_id.as_deref(), Some("719523767693"));
                assert_eq!(s.shop_name.as_deref(), Some("SCShop_Aparelli_NewBabbage"));
                assert_eq!(s.price, Some(2200)); // "2200.000000" -> 2200 aUEC
                assert_eq!(s.item_class.as_deref(), Some("33dabc-guid"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_modern_shop_ui_buy_variant() {
        // The CEntityComponentShopUIProvider shell-tag variant, same body.
        let line = "<2026-07-16T20:46:30.783Z> [Notice] <CEntityComponentShopUIProvider::SendShopBuyRequest> Sending SShopBuyRequest - playerId[204056438813] shopId[719523768273] shopName[SCShop_Centermass_NewBabbage] kioskId[719523768261] client_price[66811.000000] itemClassGUID[abc] [Team_CoreGameplayFeatures][Shopping]";
        let p = structural_parse(line).unwrap();
        match classify(&p).unwrap() {
            GameEvent::ShopBuyRequest(s) => {
                assert_eq!(s.shop_name.as_deref(), Some("SCShop_Centermass_NewBabbage"));
                assert_eq!(s.price, Some(66811));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_shop_flow_response_success() {
        let line = "<2026-05-07T13:00:01.000Z> [Notice] ShopFlowResponse(shopId=area18_kiosk_07, Result[Success])";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::ShopFlowResponse(s) => {
                assert_eq!(s.shop_id.as_deref(), Some("area18_kiosk_07"));
                assert_eq!(s.success, Some(true));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_commodity_buy_request() {
        let line = "<2026-05-07T14:00:00.000Z> [Notice] SendCommodityBuyRequest(commodity=Agricium, quantity=125.5, terminal=cru_l1)";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::CommodityBuyRequest(c) => {
                assert_eq!(c.commodity.as_deref(), Some("Agricium"));
                assert_eq!(c.quantity, Some(125.5));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_commodity_sell_request() {
        let line = "<2026-05-07T15:00:00.000Z> [Notice] SendCommoditySellRequest(commodity=Quantanium, amount=64)";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::CommoditySellRequest(c) => {
                assert_eq!(c.commodity.as_deref(), Some("Quantanium"));
                assert_eq!(c.quantity, Some(64.0));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_session_end_via_system_quit_shell() {
        let line = "<2026-05-07T16:00:00.000Z> [Notice] <SystemQuit> shutting down [Team_System]";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::SessionEnd(s) => {
                assert_eq!(s.kind, SessionEndKind::SystemQuit);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn classifies_session_end_via_fast_shutdown_body_prefix() {
        let line = "<2026-05-07T16:00:01.000Z> [Notice] CCIGBroker::FastShutdown() invoked";
        let p = structural_parse(line).unwrap();
        let event = classify(&p).unwrap();
        match event {
            GameEvent::SessionEnd(s) => {
                assert_eq!(s.kind, SessionEndKind::FastShutdown);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn body_prefix_returns_none_for_unrelated_lines() {
        // Sanity: a body-only line with no recognised keyword must
        // not produce a spurious event.
        let line =
            "<2026-05-07T17:00:00.000Z> [Notice] some unrelated diagnostic chatter, no keyword";
        let p = structural_parse(line).unwrap();
        assert!(classify(&p).is_none());
    }

    #[test]
    fn classify_with_metadata_stamps_primary_entity() {
        // Reuse the same modern PlayerDeath fixture exercised by
        // `classifies_modern_player_death_from_real_capture` — the
        // helper must produce both the event and the default Observed
        // metadata in lock-step.
        let line = "<2026-05-01T18:46:15.085Z> [Notice] <Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]> Item 'body_01_noMagicPocket_9754924365641 - Class(body_01_noMagicPocket) - Context(Streamable Runtime-spawned) - Socpak()', Recorded data is: Port Name 'Body_ItemPort', Class GUID: 'dbaa8a7d-755f-4104-8b24-7b58fd1e76f6', KeptId: '9754924365641' [Team_CoreGameplayFeatures][Unknown]";
        let parsed = structural_parse(line).expect("structural parse should succeed");
        let (event, meta) = classify_with_metadata(&parsed, Some("CommanderJim"));
        let m = meta.expect("PlayerDeath line should classify");
        assert!(event.is_some());
        assert_eq!(m.primary_entity.kind, crate::metadata::EntityKind::Player);
        assert_eq!(m.primary_entity.id, "CommanderJim");
        assert_eq!(m.source, crate::metadata::EventSource::Observed);
        assert!((m.confidence - 1.0).abs() < f32::EPSILON);
        assert!(m.group_key.starts_with("player_death:player:"));
    }

    #[test]
    fn classify_with_metadata_returns_none_for_unclassified_line() {
        let line =
            "<2026-05-17T00:00:00.000Z> [Notice] some unrelated diagnostic chatter, no keyword";
        let parsed = structural_parse(line).expect("structural parse should succeed");
        let (event, meta) = classify_with_metadata(&parsed, None);
        assert!(event.is_none());
        assert!(meta.is_none());
    }

    #[test]
    fn classify_or_capture_returns_classified_for_known_line() {
        // Reuse the modern PlayerDeath fixture from this file's own
        // classifier tests — a built-in match short-circuits before
        // any capture work happens.
        let line = "<2026-05-01T18:46:15.085Z> [Notice] <Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]> Item 'body_01_noMagicPocket_9754924365641 - Class(body_01_noMagicPocket) - Context(Streamable Runtime-spawned) - Socpak()', Recorded data is: Port Name 'Body_ItemPort', Class GUID: 'dbaa8a7d-755f-4104-8b24-7b58fd1e76f6', KeptId: '9754924365641' [Team_CoreGameplayFeatures][Unknown]";
        let parsed = structural_parse(line).expect("structural parse");
        let ctx = crate::unknown_lines::CaptureContextOwned::default();
        let outcome = classify_or_capture(&parsed, &[], &ctx, line, "2026-05-01T18:46:15Z");
        match outcome {
            ClassifyOutcome::Classified(GameEvent::PlayerDeath(_)) => {}
            other => panic!("expected Classified(PlayerDeath), got {other:?}"),
        }
    }

    #[test]
    fn classify_or_capture_returns_unknown_for_mystery_line() {
        // Shell parses fine, but no built-in matches and no remote
        // rule provided — the line is captured for review.
        let line = "<2026-05-17T14:02:30Z> [Notice] <NewMysteryEvent> something something [54324]";
        let parsed = structural_parse(line).expect("structural parse");
        let ctx = crate::unknown_lines::CaptureContextOwned::default();
        let outcome = classify_or_capture(&parsed, &[], &ctx, line, "2026-05-17T14:02:30Z");
        let captured = match outcome {
            ClassifyOutcome::Unknown(u) => u,
            other => panic!("expected Unknown, got {other:?}"),
        };
        assert!(captured.interest_score > 0);
        assert_eq!(captured.shell_tag.as_deref(), Some("NewMysteryEvent"));
        assert_eq!(captured.raw_line, line);
    }

    #[test]
    fn classify_or_capture_does_not_capture_when_remote_match_succeeds() {
        use crate::parser_defs::{compile_rules, RemoteRule, RuleMatchKind};
        // Remote rule matches "PlayerDance" lines; the same line is not
        // a built-in, so the remote arm fires and we never capture.
        let rules = vec![RemoteRule {
            id: "r-dance".to_string(),
            event_name: "PlayerDance".to_string(),
            match_kind: RuleMatchKind::EventName,
            body_regex: r"emote=(?P<emote>\w+)".to_string(),
            fields: vec!["emote".to_string()],
        }];
        let (compiled, bad) = compile_rules(&rules);
        assert!(bad.is_empty());
        let line = "<2026-05-17T15:00:00.000Z> [Notice] <PlayerDance> emote=salute [Team_X]";
        let parsed = structural_parse(line).expect("structural parse");
        let ctx = crate::unknown_lines::CaptureContextOwned::default();
        let outcome = classify_or_capture(&parsed, &compiled, &ctx, line, "2026-05-17T15:00:00Z");
        match outcome {
            ClassifyOutcome::RemoteMatched(GameEvent::RemoteMatch(m)) => {
                assert_eq!(m.event_name, "PlayerDance");
            }
            other => panic!("expected RemoteMatched, got {other:?}"),
        }
    }
}
