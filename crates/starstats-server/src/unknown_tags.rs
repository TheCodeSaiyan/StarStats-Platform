//! Opt-in unknown shell-tag sightings, and correlation against parser-health
//! findings.
//!
//! [`crate::parser_health`] detects that an event type has gone dark. It
//! cannot say why: the tag that replaced the old one lives only in the tray's
//! local `unknown_lines` queue. This module receives that tag — and ONLY the
//! tag — so a finding can name its likely cause.
//!
//! Privacy posture: engine symbol names, never line bodies. [`valid_shell_tag`]
//! is the gate, applied on the tray before send AND here on receipt. Two
//! independent checks because the first protects the user's intent and the
//! second protects the server from a modified client.
//!
//! This also makes partial collapses detectable on a small fleet. The
//! simultaneity signal in Spec 1 needs a population to mean anything; a tag
//! appearing exactly when a type died is positive evidence that does not.

use crate::repo::RepoError;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;

/// Longest shell tag we will store. Real tags are well under this; the cap
/// exists so a malformed client cannot write unbounded rows.
pub const SHELL_TAG_MAX_LEN: usize = 200;
/// Most sightings accepted in one request.
pub const MAX_TAGS_PER_BATCH: usize = 500;
/// How far before a type's last event a tag may first appear and still be
/// considered a candidate replacement. Generous on the early side: the new
/// tag often shows up in the same patch, slightly before the old one stops.
pub const CANDIDATE_LOOKBACK_DAYS: i64 = 3;
/// How far after. Wider, because a type can die on a patch the user only
/// encounters days later.
pub const CANDIDATE_LOOKAHEAD_DAYS: i64 = 21;

/// Whether a string is an acceptable shell tag.
///
/// Deliberately conservative. Observed real tags are engine symbols —
/// `InventoryManagement`, `CObjectiveMarkerComponent::AddToPlayerDataBank`,
/// `LandingArea_UnregisterFromExternalSystems_StowingVehicle` — plus a few
/// human-readable status strings with spaces and punctuation. What must never
/// arrive is a raw log body carrying player data, so the charset excludes the
/// bracket/brace/quote characters that structured payloads and identifiers use.
pub fn valid_shell_tag(s: &str) -> bool {
    if s.is_empty() || s.len() > SHELL_TAG_MAX_LEN {
        return false;
    }
    // No control characters, and none of the delimiters that appear in log
    // bodies (`[id]`, `{guid}`, quoted strings, `<Event>` shells).
    if s.chars().any(|c| {
        c.is_control()
            || matches!(
                c,
                '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | '\\' | '|'
            )
    }) {
        return false;
    }
    // Must contain at least one alphanumeric — rejects punctuation-only junk.
    s.chars().any(|c| c.is_ascii_alphanumeric())
}

/// One reported sighting, as it arrives on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TagSighting {
    pub shell_tag: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub occurrences: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_build: Option<String>,
}

/// A tag proposed as the cause of a finding, aggregated across contributors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TagCandidate {
    pub shell_tag: String,
    pub first_seen: DateTime<Utc>,
    pub occurrences: i64,
    /// How many distinct handles reported this tag. More reporters is
    /// stronger evidence, exactly as with the simultaneity signal.
    pub handle_count: i64,
}

/// Drop sightings that fail validation, returning the survivors plus the
/// number rejected.
///
/// Rejecting individually rather than failing the whole batch is deliberate:
/// one malformed tag must not cost the user every other sighting in the
/// request, and a client that starts emitting junk should degrade rather than
/// stop reporting entirely.
pub fn sanitise(batch: Vec<TagSighting>) -> (Vec<TagSighting>, usize) {
    let mut kept = Vec::with_capacity(batch.len().min(MAX_TAGS_PER_BATCH));
    let mut rejected = 0usize;
    for s in batch.into_iter().take(MAX_TAGS_PER_BATCH) {
        let sane_window = s.first_seen <= s.last_seen;
        let sane_count = s.occurrences >= 0;
        let sane_build = s
            .game_build
            .as_deref()
            .map(|b| b.len() <= 64 && !b.chars().any(char::is_control))
            .unwrap_or(true);
        if valid_shell_tag(&s.shell_tag) && sane_window && sane_count && sane_build {
            kept.push(s);
        } else {
            rejected += 1;
        }
    }
    (kept, rejected)
}

/// Inclusive window in which a tag's first sighting counts as a candidate
/// cause for a type that last fired at `last_event_at`.
pub fn candidate_window(last_event_at: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    (
        last_event_at - Duration::days(CANDIDATE_LOOKBACK_DAYS),
        last_event_at + Duration::days(CANDIDATE_LOOKAHEAD_DAYS),
    )
}

#[async_trait]
pub trait UnknownTagStore: Send + Sync + 'static {
    /// Upsert a batch for one handle. Returns how many rows were written.
    async fn record(&self, handle: &str, sightings: &[TagSighting]) -> Result<u64, RepoError>;

    /// Tags first seen inside the window, aggregated across handles, most
    /// reported first.
    async fn candidates(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<TagCandidate>, RepoError>;
}

pub struct PostgresUnknownTagStore {
    pool: PgPool,
}

impl PostgresUnknownTagStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UnknownTagStore for PostgresUnknownTagStore {
    async fn record(&self, handle: &str, sightings: &[TagSighting]) -> Result<u64, RepoError> {
        if sightings.is_empty() {
            return Ok(0);
        }
        let handle = handle.to_lowercase();
        let mut written = 0u64;
        for s in sightings {
            let n = sqlx::query(
                r#"
                INSERT INTO unknown_tag_sighting
                    (claimed_handle, shell_tag, first_seen, last_seen, occurrences, game_build)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (claimed_handle, shell_tag) DO UPDATE SET
                    -- Keep the EARLIEST first_seen: that timestamp is the
                    -- whole point of the correlation, and a later report
                    -- must never push it forward.
                    first_seen  = LEAST(unknown_tag_sighting.first_seen, EXCLUDED.first_seen),
                    last_seen   = GREATEST(unknown_tag_sighting.last_seen, EXCLUDED.last_seen),
                    occurrences = GREATEST(unknown_tag_sighting.occurrences, EXCLUDED.occurrences),
                    game_build  = COALESCE(EXCLUDED.game_build, unknown_tag_sighting.game_build),
                    updated_at  = now()
                "#,
            )
            .bind(&handle)
            .bind(&s.shell_tag)
            .bind(s.first_seen)
            .bind(s.last_seen)
            .bind(s.occurrences)
            .bind(s.game_build.as_deref())
            .execute(&self.pool)
            .await?
            .rows_affected();
            written += n;
        }
        Ok(written)
    }

    async fn candidates(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<TagCandidate>, RepoError> {
        let rows: Vec<(String, DateTime<Utc>, i64, i64)> = sqlx::query_as(
            r#"
            SELECT shell_tag,
                   MIN(first_seen)          AS first_seen,
                   SUM(occurrences)::BIGINT AS occurrences,
                   COUNT(*)::BIGINT         AS handle_count
            FROM unknown_tag_sighting
            WHERE first_seen >= $1 AND first_seen <= $2
            GROUP BY shell_tag
            ORDER BY handle_count DESC, occurrences DESC, shell_tag
            LIMIT $3
            "#,
        )
        .bind(from)
        .bind(to)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(shell_tag, first_seen, occurrences, handle_count)| TagCandidate {
                    shell_tag,
                    first_seen,
                    occurrences,
                    handle_count,
                },
            )
            .collect())
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryUnknownTagStore {
        rows: Mutex<HashMap<(String, String), TagSighting>>,
    }

    impl MemoryUnknownTagStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl UnknownTagStore for MemoryUnknownTagStore {
        async fn record(&self, handle: &str, sightings: &[TagSighting]) -> Result<u64, RepoError> {
            let mut rows = self.rows.lock().unwrap();
            let handle = handle.to_lowercase();
            for s in sightings {
                let key = (handle.clone(), s.shell_tag.clone());
                rows.entry(key)
                    .and_modify(|e| {
                        e.first_seen = e.first_seen.min(s.first_seen);
                        e.last_seen = e.last_seen.max(s.last_seen);
                        e.occurrences = e.occurrences.max(s.occurrences);
                        if s.game_build.is_some() {
                            e.game_build = s.game_build.clone();
                        }
                    })
                    .or_insert_with(|| s.clone());
            }
            Ok(sightings.len() as u64)
        }

        async fn candidates(
            &self,
            from: DateTime<Utc>,
            to: DateTime<Utc>,
            limit: i64,
        ) -> Result<Vec<TagCandidate>, RepoError> {
            let rows = self.rows.lock().unwrap();
            let mut agg: HashMap<String, TagCandidate> = HashMap::new();
            for ((_, tag), s) in rows.iter() {
                if s.first_seen < from || s.first_seen > to {
                    continue;
                }
                agg.entry(tag.clone())
                    .and_modify(|c| {
                        c.first_seen = c.first_seen.min(s.first_seen);
                        c.occurrences += s.occurrences;
                        c.handle_count += 1;
                    })
                    .or_insert_with(|| TagCandidate {
                        shell_tag: tag.clone(),
                        first_seen: s.first_seen,
                        occurrences: s.occurrences,
                        handle_count: 1,
                    });
            }
            let mut out: Vec<TagCandidate> = agg.into_values().collect();
            out.sort_by(|a, b| {
                b.handle_count
                    .cmp(&a.handle_count)
                    .then(b.occurrences.cmp(&a.occurrences))
                    .then(a.shell_tag.cmp(&b.shell_tag))
            });
            out.truncate(limit.max(0) as usize);
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryUnknownTagStore;
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn sighting(tag: &str, first: &str, occ: i64) -> TagSighting {
        TagSighting {
            shell_tag: tag.to_string(),
            first_seen: ts(first),
            last_seen: ts(first),
            occurrences: occ,
            game_build: None,
        }
    }

    #[test]
    fn accepts_real_observed_shell_tags() {
        for t in [
            "LandingArea_UnregisterFromExternalSystems_StowingVehicle",
            "CLandingArea::UnregisterFromExternalSystems",
            "InventoryManagement",
            "Local Route Guard - Server Rerouted",
            "Failed to get starmap route data!",
            "SetSalvageRepairAmmoCount_NoTarget",
        ] {
            assert!(valid_shell_tag(t), "should accept: {t}");
        }
    }

    #[test]
    fn rejects_anything_shaped_like_a_log_body() {
        // The privacy contract: tags only, never a line that could carry
        // player identifiers or coordinates.
        for t in [
            "",
            "[STOWING ON UNREGISTER] LandingArea_X [745597122922]",
            "Adding non kept item [CSCActorCorpseUtils::PopulateItemPort]",
            "<CLandingArea::UnregisterFromExternalSystems>",
            "player {A27E3980-7BC8-42F5-A348-32E97E567C8B}",
            "name=\"SomePlayer\"",
            "---",
            "tab\there",
        ] {
            assert!(!valid_shell_tag(t), "should reject: {t:?}");
        }
        assert!(!valid_shell_tag(&"x".repeat(SHELL_TAG_MAX_LEN + 1)));
    }

    #[test]
    fn sanitise_drops_bad_entries_without_losing_good_ones() {
        let batch = vec![
            sighting("GoodTag", "2026-07-16T00:00:00Z", 10),
            sighting("bad [body] tag", "2026-07-16T00:00:00Z", 10),
            sighting("AlsoGood", "2026-07-16T00:00:00Z", 5),
        ];

        let (kept, rejected) = sanitise(batch);

        assert_eq!(rejected, 1);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].shell_tag, "GoodTag");
    }

    #[test]
    fn sanitise_rejects_an_inverted_window_and_negative_counts() {
        let mut inverted = sighting("Tag", "2026-07-16T00:00:00Z", 1);
        inverted.last_seen = ts("2026-07-01T00:00:00Z");
        let mut negative = sighting("Other", "2026-07-16T00:00:00Z", -5);
        negative.last_seen = ts("2026-07-17T00:00:00Z");

        let (kept, rejected) = sanitise(vec![inverted, negative]);

        assert_eq!(kept.len(), 0);
        assert_eq!(rejected, 2);
    }

    #[test]
    fn sanitise_caps_the_batch() {
        let batch: Vec<TagSighting> = (0..MAX_TAGS_PER_BATCH + 50)
            .map(|i| sighting(&format!("Tag{i}"), "2026-07-16T00:00:00Z", 1))
            .collect();

        let (kept, _) = sanitise(batch);

        assert_eq!(kept.len(), MAX_TAGS_PER_BATCH);
    }

    #[test]
    fn candidate_window_brackets_the_collapse_moment() {
        let (from, to) = candidate_window(ts("2026-07-14T21:13:51Z"));
        // The real replacement tag first appeared 2026-07-16 — inside.
        assert!(from < ts("2026-07-16T00:00:00Z"));
        assert!(to > ts("2026-07-16T00:00:00Z"));
        // A tag from months earlier is not a candidate.
        assert!(from > ts("2026-06-01T00:00:00Z"));
    }

    #[tokio::test]
    async fn record_keeps_the_earliest_first_seen() {
        // first_seen IS the correlation signal; a later report must never
        // push it forward or the tag stops matching its own finding.
        let store = MemoryUnknownTagStore::new();
        store
            .record("nigel", &[sighting("Tag", "2026-07-16T00:00:00Z", 10)])
            .await
            .unwrap();
        store
            .record("nigel", &[sighting("Tag", "2026-07-20T00:00:00Z", 40)])
            .await
            .unwrap();

        let c = store
            .candidates(ts("2026-07-01T00:00:00Z"), ts("2026-08-01T00:00:00Z"), 10)
            .await
            .unwrap();

        assert_eq!(c.len(), 1);
        assert_eq!(c[0].first_seen, ts("2026-07-16T00:00:00Z"));
        assert_eq!(c[0].occurrences, 40);
    }

    #[tokio::test]
    async fn candidates_rank_by_reporter_count_then_volume() {
        let store = MemoryUnknownTagStore::new();
        // Reported by two handles, lower volume.
        for h in ["alice", "bob"] {
            store
                .record(h, &[sighting("WidelySeen", "2026-07-16T00:00:00Z", 10)])
                .await
                .unwrap();
        }
        // One handle, much higher volume.
        store
            .record(
                "alice",
                &[sighting("LoudButLocal", "2026-07-16T00:00:00Z", 9_000)],
            )
            .await
            .unwrap();

        let c = store
            .candidates(ts("2026-07-01T00:00:00Z"), ts("2026-08-01T00:00:00Z"), 10)
            .await
            .unwrap();

        assert_eq!(c[0].shell_tag, "WidelySeen", "breadth beats volume");
        assert_eq!(c[0].handle_count, 2);
    }

    #[tokio::test]
    async fn candidates_exclude_tags_outside_the_window() {
        let store = MemoryUnknownTagStore::new();
        store
            .record("nigel", &[sighting("Ancient", "2026-01-01T00:00:00Z", 999)])
            .await
            .unwrap();
        store
            .record("nigel", &[sighting("Fresh", "2026-07-16T00:00:00Z", 1)])
            .await
            .unwrap();

        let (from, to) = candidate_window(ts("2026-07-14T21:13:51Z"));
        let c = store.candidates(from, to, 10).await.unwrap();

        assert_eq!(c.len(), 1);
        assert_eq!(c[0].shell_tag, "Fresh");
    }

    #[tokio::test]
    async fn recording_an_empty_batch_is_a_no_op() {
        let store = MemoryUnknownTagStore::new();
        assert_eq!(store.record("nigel", &[]).await.unwrap(), 0);
    }
}
