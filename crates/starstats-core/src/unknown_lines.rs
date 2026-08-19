//! Capture and characterise lines that didn't match any built-in
//! classifier OR a remote parser rule. These flow to a local review
//! queue in the tray; the user opts in to submitting promising ones
//! back to the rule-author moderation queue on the server.
//!
//! Three concerns split across this module:
//!
//! 1. **Shape normalisation** — collapse a raw line down to a template
//!    with identifiers replaced by placeholder tokens. Same template,
//!    same `shape_hash`, so the tray can dedupe spam.
//! 2. **Interest score** — heuristic 0..=100 ranking how likely the
//!    line carries useful event signal. (Task 30.)
//! 3. **PII detection** — pre-flag handles, shard IDs, GEIDs, IPs so
//!    the user reviews redaction before submission. (Task 31.)
//!
//! Submission itself is *not* in this module — that lives in the tray
//! crate (Phase 4.B) and the server endpoint (Phase 4.C). This module
//! is pure types + functions, callable from any consumer.

use crate::wire::LogSource;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

/// One pre-existing-shape-normalised candidate.
///
/// Stored locally in the tray's SQLite cache (one row per `shape_hash`,
/// with `occurrence_count` tracking how many raw lines collapsed to
/// the same shape this session). Never auto-uploaded — only the user
/// can submit, and only after redaction review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownLine {
    pub id: String,
    pub raw_line: String,
    pub timestamp: Option<String>,
    pub shell_tag: Option<String>,
    pub partial_structured: BTreeMap<String, String>,
    /// Last 5 lines before this one in source order.
    pub context_before: Vec<String>,
    /// Up to 5 lines after — filled lazily as they arrive.
    pub context_after: Vec<String>,
    pub game_build: Option<String>,
    pub channel: LogSource,
    pub interest_score: u8,
    pub shape_hash: String,
    pub occurrence_count: u32,
    pub first_seen: String,
    pub last_seen: String,
    pub detected_pii: Vec<PiiToken>,
    pub dismissed: bool,
}

/// One auto-detected sensitive token in a raw line. Filled in by
/// [`detect_pii`] (Task 31); declared here so `UnknownLine` carries
/// the field on the wire from day one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PiiToken {
    pub kind: PiiKind,
    pub start: usize,
    pub end: usize,
    pub suggested_redaction: String,
    pub default_redact: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiKind {
    OwnHandle,
    FriendHandle,
    ShardId,
    Geid,
    IpPort,
}

// ─── Shape normalisation ────────────────────────────────────────────

static TS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z?").expect("TS_RE compiles")
});
static UUID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
        .expect("UUID_RE compiles")
});
static GEID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[\d{5,}\]").expect("GEID_RE compiles"));
static IPPORT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(?::\d{1,5})?\b").expect("IPPORT_RE compiles")
});
static QUOTED_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#""[^"]+""#).expect("QUOTED_RE compiles"));
/// Single-quoted values — overwhelmingly the player handle. The corpus
/// writes `for 'TheCodeSaiyan'`, which the double-quote rule never
/// matched, so every player produced their own shapes.
static SQUOTED_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"'[^']+'").expect("SQUOTED_RE compiles"));
/// A bracketed integer of ANY length: `Request[1]`, `Item[0]`.
///
/// `GEID_RE` only matches 5+ digits, so short indices survived into the
/// shape and split one template into a shape per index. Measured
/// 2026-07-31: `InventoryManagement` alone held 70,535 shapes for
/// 120,855 occurrences, and 74% of all shapes in the table were seen
/// exactly once — the signature of un-normalised operands.
static BRACKET_INT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\d+\]").expect("BRACKET_INT_RE compiles"));
/// A bracketed COMPOUND identifier — `[204056438813:Location:3170699229]`.
///
/// Requires at least one digit run and a colon, so it cannot match an
/// enum-ish `[Succeed]` or `[QueryOrCreate]`. That restraint is the
/// point: `Result[Succeed]` and `Result[Failed]` are genuinely different
/// templates and must not be collapsed into one another.
static BRACKET_COMPOUND_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[[A-Za-z0-9_]*\d[A-Za-z0-9_]*(?::[A-Za-z0-9_]+)+\]")
        .expect("BRACKET_COMPOUND_RE compiles")
});
static SHARD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"shard[\[\s=:]+([A-Za-z0-9_-]+)").expect("SHARD_RE compiles"));

/// Collapse a raw log line to its shape: identifiers and timestamps
/// become tokens like `<TS>`, `<GEID>`, etc. Same template → same shape.
pub fn shape_of(line: &str) -> String {
    let s = TS_RE.replace_all(line, "<TS>");
    let s = UUID_RE.replace_all(&s, "<UUID>");
    let s = GEID_RE.replace_all(&s, "[<GEID>]");
    let s = IPPORT_RE.replace_all(&s, "<IPPORT>");
    let s = QUOTED_RE.replace_all(&s, "\"<STR>\"");
    let s = SQUOTED_RE.replace_all(&s, "'<STR>'");
    // AFTER the GEID rule, so long ids keep their more specific token
    // and only the short indices GEID_RE skipped land here.
    let s = BRACKET_COMPOUND_RE.replace_all(&s, "[<ID>]");
    let s = BRACKET_INT_RE.replace_all(&s, "[<N>]");
    s.into_owned()
}

/// Stable hash of a shape, fitting in a SQLite TEXT column. The `sh_`
/// prefix makes it self-describing in row dumps; the 16 hex chars come
/// from `DefaultHasher` which is good enough for dedupe (not crypto).
pub fn shape_hash(line: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    shape_of(line).hash(&mut h);
    format!("sh_{:016x}", h.finish())
}

/// Second-level normalisation on top of [`shape_of`]: collapse the
/// value-ish tokens `shape_of` leaves distinct — bare numbers to
/// `<NUM>` and code-identifier tokens to `<SYM>` — so near-duplicate
/// shapes (same event, different vehicle class / number / enum) share
/// one coarse shape. Deliberately conservative: plain lowercase and
/// single Titlecase words (message keywords like `Notice`, `Kill`,
/// `Spawn`) are kept as-is, so genuinely different templates stay
/// apart. Existing placeholders (`<TS>`, `[<GEID>]`, `"<STR>"`, …) are
/// preserved verbatim — this only operates on whitespace tokens that
/// don't already contain a placeholder delimiter.
pub fn coarse_shape_of(line: &str) -> String {
    shape_of(line)
        .split(' ')
        .map(coarse_token)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Coarsen one whitespace-delimited token. Tokens containing a
/// placeholder delimiter (`<`, `>`, `[`, `]`, `"`) are returned
/// unchanged so `shape_of`'s placeholders survive untouched. Otherwise
/// leading/trailing punctuation is preserved and only the
/// alphanumeric core (which may itself contain an internal `::`, e.g.
/// `CEntity::Kill`) is classified.
fn coarse_token(tok: &str) -> String {
    if tok.contains(['<', '>', '[', ']', '"']) {
        return tok.to_string();
    }
    let Some(start) = tok.find(|c: char| c.is_alphanumeric() || c == '_') else {
        return tok.to_string(); // pure punctuation / empty
    };
    // `rfind` returns the START byte of the last matching char, which is
    // only a valid char-boundary end when that char is single-byte ASCII.
    // A token ending in a multibyte char (e.g. `café`, `Москва`, `日本語`)
    // would make the `+ 1` land mid-codepoint and panic in `split_at`
    // below. Walk from the end instead and add the matched char's own
    // UTF-8 length to get a boundary that's always valid.
    let end = tok
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric() || *c == '_')
        .map(|(i, c)| i + c.len_utf8())
        .expect("start already found a matching char, so at least one exists");
    let (prefix, rest) = tok.split_at(start);
    let (core, suffix) = rest.split_at(end - start);
    format!("{prefix}{}{suffix}", classify_namespaced(core))
}

/// Classify a token's alphanumeric core, treating `::`-separated
/// segments (e.g. `CEntity::Kill`) independently rather than as one
/// atomic identifier. This is the key move that keeps `CEntity::Kill`
/// and `CEntity::Spawn` distinct: `CEntity` is identifier-shaped
/// (inner uppercase) and collapses to `<SYM>`, but `Kill`/`Spawn` are
/// single Titlecase message keywords and are kept, so the two lines
/// still diverge after coarsening.
fn classify_namespaced(core: &str) -> String {
    core.split("::")
        .map(classify_core)
        .collect::<Vec<_>>()
        .join("::")
}

/// `<NUM>` for a pure number, `<SYM>` for a code-identifier segment,
/// else the segment unchanged. Code-identifier = contains `_`, OR
/// mixes letters and digits, OR is CamelCase (has an uppercase letter
/// after its first character), OR is ALL-CAPS of length >= 2. Plain
/// lowercase words and single Titlecase words fail all four checks
/// and are kept, preserving distinct message keywords.
fn classify_core(core: &str) -> String {
    let is_number = !core.is_empty()
        && core.chars().all(|c| c.is_ascii_digit() || c == '.')
        && core.chars().any(|c| c.is_ascii_digit());
    if is_number {
        return "<NUM>".to_string();
    }
    let has_underscore = core.contains('_');
    let has_letter = core.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = core.chars().any(|c| c.is_ascii_digit());
    let inner_upper = core.chars().skip(1).any(|c| c.is_ascii_uppercase());
    let all_caps = core.len() >= 2 && core.chars().all(|c| c.is_ascii_uppercase() || c == '_');
    let is_identifier = has_letter && (has_underscore || has_digit || inner_upper || all_caps);
    if is_identifier {
        "<SYM>".to_string()
    } else {
        core.to_string()
    }
}

/// Coarse-shape counterpart to [`shape_hash`]. Distinct `csh_` prefix
/// so the two hash families are never confused in a row dump or a
/// cache key.
pub fn coarse_shape_hash(line: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    coarse_shape_of(line).hash(&mut h);
    format!("csh_{:016x}", h.finish())
}

// ─── Interest score ─────────────────────────────────────────────────

/// Borrowed context for [`interest_score`]. Caller owns the HashSets;
/// this struct keeps the function signature short and lets us add new
/// inputs later without breaking call sites that build from a
/// long-lived runtime cache.
pub struct InterestContext<'a> {
    /// Shell tags the parser has built-in or remote rules for.
    pub known_shell_tags: &'a HashSet<String>,
    /// Subset of known shell tags that have at least one remote rule
    /// targeting them. Tags in `known_shell_tags` but not here are
    /// tags the parser knows about but doesn't classify (yet).
    pub known_rule_tags: &'a HashSet<String>,
    pub session_occurrence_count: u32,
    pub multi_session: bool,
    pub already_remote_matched: bool,
}

/// Heuristic 0..=100 for how likely a line carries useful event
/// signal. The UI surfaces lines above a configurable threshold
/// (default 50). Tuning knobs:
///
/// * Unknown shell tag → +40 (strongest single signal).
/// * Known shell tag with no remote rule → +30 (gap in coverage).
/// * GEID-shaped digit cluster in body → +15.
/// * Body keywords (`OOC_`, `body_`, `_class`) → +10.
/// * Repeated this session → +10, multi-session → +20 (sustained, not
///   one-off noise).
/// * Extremely short or long → −30 (not an event we can usefully
///   parse).
/// * `already_remote_matched` short-circuits to 0 — a matched line is
///   not unknown.
pub fn interest_score(line: &str, shell_tag: Option<&str>, ctx: &InterestContext) -> u8 {
    if ctx.already_remote_matched {
        return 0;
    }
    let mut score: i32 = 0;
    if let Some(tag) = shell_tag {
        if !ctx.known_shell_tags.contains(tag) {
            score += 40;
        } else if !ctx.known_rule_tags.contains(tag) {
            score += 30;
        }
    }
    if line.contains('[') && line.chars().filter(|c| c.is_ascii_digit()).count() >= 5 {
        score += 15;
    }
    if line.contains("OOC_") || line.contains("body_") || line.contains("_class") {
        score += 10;
    }
    if ctx.session_occurrence_count >= 3 {
        score += 10;
    }
    if ctx.multi_session {
        score += 20;
    }
    let len = line.len();
    if !(20..=2000).contains(&len) {
        score -= 30;
    }
    score.clamp(0, 100) as u8
}

// ─── PII detection ──────────────────────────────────────────────────

/// Auto-detect potentially sensitive tokens in a raw line. The detector
/// errs on the side of false positives — the user reviews and toggles
/// per-token before submission. `default_redact = true` for the
/// player's own handle and shard ID (high re-identification risk);
/// other kinds default off because they're often part of the signal
/// the rule author needs to see.
pub fn detect_pii(line: &str, own_handle: &str, known_friends: &[String]) -> Vec<PiiToken> {
    let mut tokens = Vec::new();

    if !own_handle.is_empty() {
        for idx in line.match_indices(own_handle).map(|(i, _)| i) {
            tokens.push(PiiToken {
                kind: PiiKind::OwnHandle,
                start: idx,
                end: idx + own_handle.len(),
                suggested_redaction: "[HANDLE]".into(),
                default_redact: true,
            });
        }
    }

    for friend in known_friends {
        if friend.is_empty() {
            continue;
        }
        for idx in line.match_indices(friend.as_str()).map(|(i, _)| i) {
            tokens.push(PiiToken {
                kind: PiiKind::FriendHandle,
                start: idx,
                end: idx + friend.len(),
                suggested_redaction: "[FRIEND]".into(),
                default_redact: false,
            });
        }
    }

    for m in GEID_RE.find_iter(line) {
        tokens.push(PiiToken {
            kind: PiiKind::Geid,
            start: m.start(),
            end: m.end(),
            suggested_redaction: "[GEID]".into(),
            default_redact: false,
        });
    }
    for m in IPPORT_RE.find_iter(line) {
        tokens.push(PiiToken {
            kind: PiiKind::IpPort,
            start: m.start(),
            end: m.end(),
            suggested_redaction: "[IPPORT]".into(),
            default_redact: false,
        });
    }

    for cap in SHARD_RE.captures_iter(line) {
        let m = cap
            .get(1)
            .expect("SHARD_RE capture group 1 is always present on a match");
        tokens.push(PiiToken {
            kind: PiiKind::ShardId,
            start: m.start(),
            end: m.end(),
            suggested_redaction: "[SHARD]".into(),
            default_redact: true,
        });
    }

    tokens.sort_by_key(|t| t.start);
    tokens
}

// ─── Capture entry point ────────────────────────────────────────────

/// Owned-string capture context. Built by the caller (typically the
/// tray, threaded through from the live runtime cache). Holds all the
/// inputs needed to score, redact, and stamp a raw line into an
/// [`UnknownLine`].
///
/// `channel` defaults to [`LogSource::Other`] because `LogSource`
/// itself does not implement `Default` (the wire type insists on an
/// explicit value at every other call site).
#[derive(Debug)]
pub struct CaptureContextOwned {
    pub own_handle: String,
    pub known_friends: Vec<String>,
    pub known_shell_tags: HashSet<String>,
    pub known_rule_tags: HashSet<String>,
    pub session_occurrence_count: u32,
    pub multi_session: bool,
    pub already_remote_matched: bool,
    pub game_build: Option<String>,
    pub channel: LogSource,
    pub context_before: Vec<String>,
}

impl Default for CaptureContextOwned {
    fn default() -> Self {
        Self {
            own_handle: String::new(),
            known_friends: Vec::new(),
            known_shell_tags: HashSet::new(),
            known_rule_tags: HashSet::new(),
            session_occurrence_count: 0,
            multi_session: false,
            already_remote_matched: false,
            game_build: None,
            channel: LogSource::Other,
            context_before: Vec::new(),
        }
    }
}

/// Substrings that mark a structurally-valid log line as
/// rendering/VFX-engine chatter — never a gameplay event, so never worth
/// surfacing in the parser-review queue. `[Team_VFX]` is the engine
/// subsystem tag the line carries; `Particle System` is the event-name
/// form the same noise takes (e.g. `<[Particle System] … Effect>`). Both
/// are high-precision — no gameplay event originates from these — so a
/// match is a safe drop. Extend this list as new junk families surface.
///
/// Kept as data (not a regex) so the client's SQLite purge can reuse the
/// exact same markers: one source of truth for "what is garbage".
pub const GARBAGE_LINE_MARKERS: &[&str] = &[
    "[Team_VFX]",
    "Particle System",
    // Particle/render engine chatter that slips past the tags above
    // (e.g. `<CPU Particle Limit reached>`, `<Particle Emitter Rendered
    // from multiple views!>`) — CPU Particle Limit alone is ~1/3 of all
    // captured unknown-line volume.
    "CPU Particle Limit",
    "Particle Emitter",
    // Entity / item-port / attachment streaming spam — engine internals,
    // never a gameplay event.
    "InitializeSlotMetadata",
    "DuplicatedItemPorts",
    "CAttachableComponent",
    "CreationRefusedEntity",
    "Failed attachment",
];

/// True when a line is known-garbage that must never be captured into the
/// unknown-line review queue. Checked at ingest, before capture, and
/// mirrored by the client's startup purge of already-captured rows.
pub fn is_garbage_line(line: &str) -> bool {
    GARBAGE_LINE_MARKERS.iter().any(|m| line.contains(m))
}

/// Build a captured record from one raw line + surrounding context.
/// `occurrence_count` starts at 1; the tray's SQLite layer bumps it
/// when a later line collapses to the same `shape_hash`.
pub fn capture(
    line: &str,
    shell_tag: Option<&str>,
    ctx: &CaptureContextOwned,
    now_rfc3339: &str,
) -> UnknownLine {
    let ictx = InterestContext {
        known_shell_tags: &ctx.known_shell_tags,
        known_rule_tags: &ctx.known_rule_tags,
        session_occurrence_count: ctx.session_occurrence_count,
        multi_session: ctx.multi_session,
        already_remote_matched: ctx.already_remote_matched,
    };
    let interest = interest_score(line, shell_tag, &ictx);
    let pii = detect_pii(line, &ctx.own_handle, &ctx.known_friends);
    UnknownLine {
        id: Uuid::new_v4().to_string(),
        raw_line: line.into(),
        timestamp: extract_timestamp(line),
        shell_tag: shell_tag.map(String::from),
        partial_structured: BTreeMap::new(),
        context_before: ctx.context_before.clone(),
        context_after: Vec::new(),
        game_build: ctx.game_build.clone(),
        channel: ctx.channel,
        interest_score: interest,
        shape_hash: shape_hash(line),
        occurrence_count: 1,
        first_seen: now_rfc3339.to_string(),
        last_seen: now_rfc3339.to_string(),
        detected_pii: pii,
        dismissed: false,
    }
}

/// Pull an ISO-8601 prefix out of the line if it's present. Best-effort.
fn extract_timestamp(line: &str) -> Option<String> {
    TS_RE.find(line).map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    /// Three REAL lines from the tray corpus, differing only in the
    /// bracketed request number. Measured 2026-07-31: the `InventoryManagement`
    /// tag alone held **70,535 distinct shapes for 120,855 occurrences** —
    /// 1.7 per shape — and 74% of ALL shapes in the table were seen exactly
    /// once. A shape hash that does not collapse these is not clustering.
    const REAL_INVENTORY_LINES: [&str; 3] = [
        "<2026-05-20T23:56:56.191Z> [Notice] <InventoryManagement> Request[1] for 'TheCodeSaiyan' [204056438813] Result[Succeed] Item[0] CanLockQueue[No]. [Team_CoreGameplayFeatures][Inventory]",
        "<2026-06-01T13:30:37.033Z> [Notice] <InventoryManagement> Request[2] for 'TheCodeSaiyan' [204056438813] Result[Succeed] Item[0] CanLockQueue[No]. [Team_CoreGameplayFeatures][Inventory]",
        "<2026-06-01T02:05:00.528Z> [Notice] <InventoryManagement> Request[3] for 'TheCodeSaiyan' [204056438813] Result[Succeed] Item[0] CanLockQueue[No]. [Team_CoreGameplayFeatures][Inventory]",
    ];

    #[test]
    fn real_lines_differing_only_by_a_bracketed_index_share_one_shape() {
        let shapes: std::collections::HashSet<String> =
            REAL_INVENTORY_LINES.iter().map(|l| shape_of(l)).collect();
        assert_eq!(
            shapes.len(),
            1,
            "same template, different Request[N] -> must be ONE shape, got {shapes:#?}"
        );
    }

    #[test]
    fn a_single_quoted_handle_is_normalised_like_a_double_quoted_one() {
        // The corpus quotes the player handle with SINGLE quotes, which
        // the double-quote rule never matched — so every player's lines
        // formed their own shapes.
        let a = shape_of("[Notice] <X> Request for 'PlayerOne' done");
        let b = shape_of("[Notice] <X> Request for 'PlayerTwo' done");
        assert_eq!(a, b, "handle must not survive into the shape");
    }

    #[test]
    fn a_bracketed_compound_id_is_normalised() {
        // Real corpus form: Inventory[204056438813:Location:3170699229].
        let a = shape_of("<X> Requesting Inventory[204056438813:Location:3170699229] ok");
        let b = shape_of("<X> Requesting Inventory[204056438814:Location:9999999999] ok");
        assert_eq!(a, b);
    }

    #[test]
    fn an_enum_like_bracketed_word_is_never_normalised() {
        // The restraint that keeps the fix honest. Collapsing these
        // would merge a success and a failure into one shape and hide
        // the distinction entirely.
        assert_ne!(
            shape_of("Result[Succeed] Item[0]"),
            shape_of("Result[Failed] Item[0]")
        );
        assert_ne!(
            shape_of("type [QueryOrCreate]"),
            shape_of("type [DeleteOnly]")
        );
    }

    #[test]
    fn distinct_templates_still_stay_apart() {
        // The whole risk of collapsing harder: over-normalising would
        // merge genuinely different events into one shape and hide them.
        let a = shape_of("[Notice] <Inventory> Request[1] Result[Succeed]");
        let b = shape_of("[Notice] <Inventory> Request[1] Result[Failed]");
        assert_ne!(a, b, "different outcomes are different templates");
    }

    use super::*;

    #[test]
    fn garbage_line_matches_vfx_and_particle_but_not_gameplay() {
        // The real particle-effect line from the sample log fixture.
        let particle = "<2026-05-02T21:15:03.053Z> [Error] <[Particle System] Kidnapping Child Effect> Expecting an independent child. [Team_VFX][VFX]";
        assert!(
            is_garbage_line(particle),
            "particle/VFX line must be garbage"
        );
        // A VFX line whose event name doesn't say "Particle" is still
        // caught by the subsystem tag.
        assert!(is_garbage_line(
            "<...> <SomeOtherThing> blah [Team_VFX][VFX]"
        ));
        // The broadened engine-noise families (verbatim from real captures).
        for junk in [
            "<2026-07-16T18:20:57.236Z> [Warning] <CPU Particle Limit reached> [Team_VFX][VFX]",
            "<2026-07-16T18:20:57.236Z> [Warning] <Particle Emitter Rendered from multiple views!> ...",
            "<..> <InitializeSlotMetadata_Failed> ...",
            "<..> <DuplicatedItemPorts> ...",
            "<..> <CAttachableComponent::GetPortFromId> ...",
            "<..> <CreationRefusedEntity> ...",
            "<..> [Error] <Failed attachment> Unable to find item port for entity",
        ] {
            assert!(is_garbage_line(junk), "engine noise must be dropped: {junk}");
        }
        // A genuine gameplay unknown must NOT be flagged.
        let gameplay = "<2026-05-01T18:46:15.085Z> [Notice] <NewMysteryEvent> player did a thing [Team_ActorFeatures][Gameplay]";
        assert!(!is_garbage_line(gameplay), "gameplay event must survive");
    }

    #[test]
    fn shape_normalises_timestamps_and_geids() {
        let line = "<2026-05-17T14:02:31.000Z> [Foo] <CargoManifestSync> for vehicle id [54324] uuid [a1b2c3d4-1234-5678-9abc-def012345678]";
        let s = shape_of(line);
        assert!(!s.contains("2026-05-17"));
        assert!(!s.contains("54324"));
        assert!(!s.contains("a1b2c3d4-1234-5678-9abc-def012345678"));
        assert!(s.contains("<CargoManifestSync>"));
        assert!(s.contains("<TS>"));
        assert!(s.contains("<GEID>"));
        assert!(s.contains("<UUID>"));
    }

    #[test]
    fn shape_stable_across_value_changes() {
        let a = shape_of("<2026-01-01T00:00:00Z> [X] <Foo> id [12345]");
        let b = shape_of("<2026-05-17T14:02:31Z> [X] <Foo> id [54324]");
        assert_eq!(a, b);
    }

    #[test]
    fn shape_hash_stable() {
        let h1 = shape_hash("<2026-01-01T00:00:00Z> [X] <Foo>");
        let h2 = shape_hash("<2026-12-31T23:59:59Z> [X] <Foo>");
        assert_eq!(h1, h2);
    }

    #[test]
    fn shape_collapses_ip_port_and_quoted_strings() {
        let a = shape_of(r#"<2026-01-01T00:00:00Z> connect 1.2.3.4:64300 name="alice""#);
        let b = shape_of(r#"<2026-12-31T23:59:59Z> connect 9.8.7.6:65000 name="bob""#);
        assert_eq!(a, b);
        assert!(a.contains("<IPPORT>"));
        assert!(a.contains("\"<STR>\""));
    }

    // ─── Coarse shape normalisation ──────────────────────────────────

    #[test]
    fn coarse_collapses_near_duplicate_shapes() {
        // Same event, different vehicle class → same coarse shape.
        let a = coarse_shape_of("2026-01-01T00:00:00Z Notice CEntity::Kill AEGS_Gladius [12345]");
        let b = coarse_shape_of("2026-01-01T00:00:00Z Notice CEntity::Kill RSI_Scorpius [67890]");
        assert_eq!(a, b, "class-name-only difference must collapse");
        // Same event, differing number.
        let c = coarse_shape_of("Notice quantum travel 42 km");
        let d = coarse_shape_of("Notice quantum travel 9001 km");
        assert_eq!(c, d, "numeric-only difference must collapse");
    }

    #[test]
    fn coarse_does_not_over_merge_distinct_templates() {
        let kill = coarse_shape_of("Notice CEntity::Kill AEGS_Gladius");
        let spawn = coarse_shape_of("Notice CEntity::Spawn AEGS_Gladius");
        assert_ne!(
            kill, spawn,
            "different lowercase/Titlecase keywords stay distinct"
        );
    }

    #[test]
    fn coarse_preserves_existing_placeholders() {
        // shape_of turns the timestamp into <TS>; coarse must NOT mangle it into <<SYM>>.
        let out = coarse_shape_of("2026-01-01T00:00:00Z Notice thing");
        assert!(
            out.contains("<TS>"),
            "placeholder <TS> preserved, got: {out}"
        );
        assert!(
            !out.contains("<<"),
            "no nested placeholder corruption, got: {out}"
        );
    }

    #[test]
    fn coarse_is_idempotent() {
        let once = coarse_shape_of("Notice CEntity::Kill AEGS_Gladius 42 [12345]");
        let twice = coarse_shape_of(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn coarse_shape_of_handles_non_ascii_tokens_without_panicking() {
        // Tokens ending in a multibyte char must not panic (byte-boundary bug).
        let out = coarse_shape_of("Notice café Москва 日本語 done");
        assert!(!out.is_empty());
        // Idempotent + stable.
        assert_eq!(coarse_shape_of(&out), out);
    }

    #[test]
    fn coarse_hash_has_distinct_prefix_and_is_stable() {
        let h = coarse_shape_hash("Notice CEntity::Kill AEGS_Gladius");
        assert!(h.starts_with("csh_"), "got {h}");
        assert_ne!(h, shape_hash("Notice CEntity::Kill AEGS_Gladius"));
        assert_eq!(h, coarse_shape_hash("Notice CEntity::Kill AEGS_Gladius"));
    }

    #[test]
    fn coarse_handles_edge_cases() {
        assert_eq!(coarse_shape_of(""), coarse_shape_of("")); // no panic
                                                              // A line already fully placeholdered stays stable.
        let ph = coarse_shape_of("<TS> Notice");
        assert_eq!(coarse_shape_of(&ph), ph);
    }

    // ─── Interest score ──────────────────────────────────────────────

    fn known_tags(tags: &[&str]) -> HashSet<String> {
        tags.iter().map(|s| (*s).to_string()).collect()
    }

    fn ctx_with<'a>(
        known_shell: &'a HashSet<String>,
        known_rule: &'a HashSet<String>,
    ) -> InterestContext<'a> {
        InterestContext {
            known_shell_tags: known_shell,
            known_rule_tags: known_rule,
            session_occurrence_count: 0,
            multi_session: false,
            already_remote_matched: false,
        }
    }

    #[test]
    fn unknown_shell_tag_surfaces_above_threshold() {
        let known_shell = HashSet::new();
        let known_rule = HashSet::new();
        let ctx = ctx_with(&known_shell, &known_rule);
        let line = "<2026-05-17T14:02:30Z> [Notice] <NewMystery> body with id [54324]";
        let score = interest_score(line, Some("NewMystery"), &ctx);
        // +40 unknown shell, +15 GEID-like cluster = 55, comfortably > 50.
        assert!(score >= 50, "expected surfacing score, got {score}");
    }

    #[test]
    fn known_tag_without_rule_scores_below_unknown() {
        let known_shell = known_tags(&["PartiallyKnown"]);
        let known_rule = HashSet::new();
        let ctx = ctx_with(&known_shell, &known_rule);
        // No '[' in the line so the GEID bonus doesn't fire — isolate the
        // +30 gap-in-coverage contribution.
        let line = "<2026-05-17T14:02:30Z> Notice PartiallyKnown short text";
        let score = interest_score(line, Some("PartiallyKnown"), &ctx);
        assert_eq!(score, 30);
    }

    #[test]
    fn already_remote_matched_short_circuits_to_zero() {
        let known_shell = HashSet::new();
        let known_rule = HashSet::new();
        let mut ctx = ctx_with(&known_shell, &known_rule);
        ctx.already_remote_matched = true;
        let line = "<2026-05-17T14:02:30Z> [Notice] <NewMystery> body with id [54324]";
        assert_eq!(interest_score(line, Some("NewMystery"), &ctx), 0);
    }

    #[test]
    fn fully_classified_tag_scores_zero() {
        let known_shell = known_tags(&["FullyKnown"]);
        let known_rule = known_tags(&["FullyKnown"]);
        let ctx = ctx_with(&known_shell, &known_rule);
        // No '[' in the line, no GEID bonus — isolate the fully-classified case.
        let line = "<2026-05-17T14:02:30Z> Notice FullyKnown some short body";
        assert_eq!(interest_score(line, Some("FullyKnown"), &ctx), 0);
    }

    #[test]
    fn repeated_lines_score_higher() {
        let known_shell = HashSet::new();
        let known_rule = HashSet::new();
        let mut ctx = ctx_with(&known_shell, &known_rule);
        ctx.session_occurrence_count = 5;
        ctx.multi_session = true;
        let line = "<2026-05-17T14:02:30Z> [Notice] <NewMystery> body with id [54324]";
        let score = interest_score(line, Some("NewMystery"), &ctx);
        // +40 unknown +15 GEID +10 repeats +20 multi-session = 85.
        assert_eq!(score, 85);
    }

    #[test]
    fn keyword_bonus_for_body_class_etc() {
        let known_shell = HashSet::new();
        let known_rule = HashSet::new();
        let ctx = ctx_with(&known_shell, &known_rule);
        let line =
            "<2026-05-17T14:02:30Z> [Notice] <NewMystery> killed body_01_noMagicPocket id [54324]";
        let score = interest_score(line, Some("NewMystery"), &ctx);
        // +40 unknown +15 GEID +10 keyword.
        assert_eq!(score, 65);
    }

    #[test]
    fn very_short_or_long_lines_penalised() {
        let known_shell = HashSet::new();
        let known_rule = HashSet::new();
        let ctx = ctx_with(&known_shell, &known_rule);
        let short = "<X> <Foo>";
        // Unknown tag (+40) but len < 20 (−30) = 10, well below threshold.
        let score = interest_score(short, Some("Foo"), &ctx);
        assert!(score < 50);
    }

    // ─── PII detection ───────────────────────────────────────────────

    #[test]
    fn detects_own_handle_with_default_redact_on() {
        let line = "Notice: TheCodeSaiyan joined the PU";
        let tokens = detect_pii(line, "TheCodeSaiyan", &[]);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, PiiKind::OwnHandle);
        assert!(tokens[0].default_redact);
        assert_eq!(&line[tokens[0].start..tokens[0].end], "TheCodeSaiyan");
    }

    #[test]
    fn detects_friend_handle_with_default_redact_off() {
        let line = "alice killed bob";
        let friends = vec!["bob".to_string()];
        let tokens = detect_pii(line, "", &friends);
        let friend = tokens
            .iter()
            .find(|t| t.kind == PiiKind::FriendHandle)
            .expect("friend handle detected");
        assert!(!friend.default_redact);
        assert_eq!(&line[friend.start..friend.end], "bob");
    }

    #[test]
    fn detects_geid_in_brackets() {
        let line = "spawned id [54324] in zone";
        let tokens = detect_pii(line, "", &[]);
        let geid = tokens
            .iter()
            .find(|t| t.kind == PiiKind::Geid)
            .expect("GEID detected");
        assert_eq!(&line[geid.start..geid.end], "[54324]");
    }

    #[test]
    fn detects_ip_port() {
        let line = "connect 1.2.3.4:64300 to shard";
        let tokens = detect_pii(line, "", &[]);
        let ip = tokens
            .iter()
            .find(|t| t.kind == PiiKind::IpPort)
            .expect("IP:port detected");
        assert_eq!(&line[ip.start..ip.end], "1.2.3.4:64300");
    }

    #[test]
    fn detects_shard_id_with_default_redact_on() {
        let line = "address[1.2.3.4] port[64300] shard[pub_euw1b] locationId[1]";
        let tokens = detect_pii(line, "", &[]);
        let shard = tokens
            .iter()
            .find(|t| t.kind == PiiKind::ShardId)
            .expect("shard id detected");
        assert!(shard.default_redact);
        assert_eq!(&line[shard.start..shard.end], "pub_euw1b");
    }

    #[test]
    fn multi_token_line_sorted_by_start() {
        let line = "TheCodeSaiyan connected to 1.2.3.4:64300 shard[pub_euw1b] id [54324]";
        let tokens = detect_pii(line, "TheCodeSaiyan", &[]);
        for win in tokens.windows(2) {
            assert!(win[0].start <= win[1].start);
        }
        let kinds: HashSet<_> = tokens.iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&PiiKind::OwnHandle));
        assert!(kinds.contains(&PiiKind::IpPort));
        assert!(kinds.contains(&PiiKind::ShardId));
        assert!(kinds.contains(&PiiKind::Geid));
    }

    // ─── Capture entry point ─────────────────────────────────────────

    #[test]
    fn capture_records_shape_and_surfacing_score() {
        let ctx = CaptureContextOwned::default();
        let line = "<2026-05-17T14:02:30Z> [Notice] <NewMystery> body with id [54324]";
        let captured = capture(line, Some("NewMystery"), &ctx, "2026-05-17T14:02:30Z");
        assert!(captured.interest_score >= 50);
        assert!(captured.shape_hash.starts_with("sh_"));
        assert_eq!(captured.shell_tag.as_deref(), Some("NewMystery"));
        assert_eq!(captured.occurrence_count, 1);
        assert!(!captured.dismissed);
    }

    #[test]
    fn capture_scores_zero_when_already_remote_matched() {
        let ctx = CaptureContextOwned {
            already_remote_matched: true,
            ..CaptureContextOwned::default()
        };
        let line = "<2026-05-17T14:02:30Z> [Notice] <NewMystery> body with id [54324]";
        let captured = capture(line, Some("NewMystery"), &ctx, "2026-05-17T14:02:30Z");
        assert_eq!(captured.interest_score, 0);
    }

    #[test]
    fn capture_extracts_timestamp_when_present() {
        let ctx = CaptureContextOwned::default();
        let line = "<2026-05-17T14:02:30Z> [Notice] <Foo> body";
        let captured = capture(line, Some("Foo"), &ctx, "2026-05-17T14:02:30Z");
        assert_eq!(captured.timestamp.as_deref(), Some("2026-05-17T14:02:30Z"));
    }

    #[test]
    fn capture_returns_no_timestamp_for_timestampless_line() {
        let ctx = CaptureContextOwned::default();
        let line = "no timestamp here at all";
        let captured = capture(line, None, &ctx, "2026-05-17T14:02:30Z");
        assert!(captured.timestamp.is_none());
    }
}
