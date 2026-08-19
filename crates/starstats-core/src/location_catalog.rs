//! In-memory index over Star Citizen location entries — the
//! catalogue surface consumed by [`crate::location_classifier`].
//!
//! Built once from the union of `api.star-citizen.wiki`'s structured
//! data (primary) and `starcitizen.tools`'s richer taxonomy
//! (enrichment), as produced by the server's
//! `reference_data.rs` + `location_enrichment.rs` cron jobs. Lives
//! in `starstats-core` so both tray (offline classification from
//! a cached snapshot) and server (online enrichment at ingest)
//! agree on the same data shape.
//!
//! Three lookup indices, all keyed on lowercase ASCII so case
//! variation in game logs (`STANTON1B` vs `Stanton1b` vs
//! `stanton1b`) never matters:
//!
//! * `by_engine_tag` — the wiki's `tag.name` field
//!   (e.g. `"Stanton1b"`). Primary join key against engine class
//!   names — the game's log lines reference exactly this token
//!   embedded in longer strings like `OOC_Stanton_1b_Aberdeen`.
//! * `by_slug` — URL-safe canonical id (e.g. `"aberdeen"`). Used
//!   when the engine string contains the wiki slug verbatim.
//! * `by_normalized_name` — display name lowercased + space-stripped
//!   (e.g. `"newbabbage"`). Last-resort match for engine strings
//!   that embed the human name directly.
//!
//! Build cost is O(N); on a ~1955-row catalogue this is sub-ms.
//! Lookups are O(1).

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::location_taxonomy::LocationTaxonomy;

/// One catalogue entry — the unified projection of an
/// `api.star-citizen.wiki` row plus its (optional) `starcitizen.tools`
/// enrichment. Serializable so the tray can ship a JSON snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationCatalogEntry {
    /// URL-safe canonical id from `api.star-citizen.wiki`. The
    /// primary join key.
    pub slug: String,
    /// Player-friendly name (e.g. `"New Babbage"`).
    pub display_name: String,
    /// Engine class name — what the wiki upstream calls
    /// `class_name` and what the in-game logs reference. Distinct
    /// from `slug` for almost every entry.
    pub class_name: String,
    /// Canonical system display (e.g. `"Stanton"`). From
    /// `metadata.star.name` on the primary source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Immediate parent body (e.g. `"Hurston"` for Aberdeen,
    /// `null` for planets and stations that orbit a star directly).
    /// From `metadata.parent.name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_body: Option<String>,
    /// Engine-internal join token (e.g. `"Stanton1b"`). The
    /// strongest match key — when present, looking up by this
    /// hits exactly one row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_tag: Option<String>,
    /// Coarse classification from `api.star-citizen.wiki`'s
    /// `type.classification` (`"Planet"`, `"Moon"`, `"Settlement"`,
    /// `"Space Station"`, …). Kept as a free-form string because
    /// the upstream allow-list shifts; the richer `taxonomy.tier`
    /// is the preferred discriminator when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    /// `starcitizen.tools` enrichment. Empty (default) when no
    /// matching wiki page exists.
    #[serde(default)]
    pub taxonomy: LocationTaxonomy,
}

impl LocationCatalogEntry {
    /// True when the entry has at least one populated source —
    /// guards against accidentally indexing all-empty rows.
    pub fn is_present(&self) -> bool {
        !self.slug.is_empty() && !self.display_name.is_empty()
    }
}

/// Read-only catalogue. Construct via [`LocationCatalog::from_entries`];
/// after that, only lookup methods are public.
///
/// Entries are wrapped in `Arc` so the three indices can share
/// pointers without cloning the row data — 1955 entries × ~400
/// bytes × 3 indices would be ~2.4 MB of duplication otherwise.
#[derive(Debug, Default)]
pub struct LocationCatalog {
    entries: Vec<Arc<LocationCatalogEntry>>,
    by_engine_tag: HashMap<String, Arc<LocationCatalogEntry>>,
    by_slug: HashMap<String, Arc<LocationCatalogEntry>>,
    by_normalized_name: HashMap<String, Arc<LocationCatalogEntry>>,
    /// Content tokens of every entry's display name, aligned by index
    /// with `entries`. Used by [`LocationCatalog::fuzzy_match`] to score
    /// token overlap. Built once; never mutated after construction.
    entry_tokens: Vec<Vec<String>>,
    /// Document frequency of each content token across all display
    /// names. Drives the inverse-document-frequency weighting — a rare
    /// word (`"kaltag"`, df 1) dominates a common one (`"outpost"`,
    /// df ~200) so a match resting on filler words scores near zero.
    name_token_df: HashMap<String, u32>,
    /// Inverted index `token → entry indices`, restricted to tokens
    /// rare enough to be worth gathering candidates on (df ≤
    /// [`MAX_INDEX_DF`]). Common filler tokens are deliberately absent —
    /// they still contribute to *scoring* (via `name_token_df`) but
    /// never *seed* a candidate set.
    name_token_index: HashMap<String, Vec<usize>>,
}

/// A token must appear in at most this many display names to seed a
/// fuzzy-match candidate set. Above this it's filler (`"outpost"`,
/// `"station"`, `"research"`) and gathering on it would scan hundreds
/// of rows for no precision gain.
const MAX_INDEX_DF: u32 = 40;

/// The *anchor* requirement: to accept a fuzzy match, the matched entry
/// must share a non-digit content token this rare with the query. Set
/// deliberately low (≤ 4) so the anchor is a near-unique *place* name
/// (`"kaltag"`, `"goldenrod"`, df 1), never a corporate operator
/// shared across a dozen sibling outposts (`"rayari"`, df 6). This is
/// the guard that rejects `RayariHydro_McGarth` (no catalogued
/// `mcgarth`) instead of letting it land on a random Rayari outpost.
/// Trade-off: digit-discriminated families whose only non-digit token
/// is a shared operator (Shubin `SAL-2`/`SAL-5`) fall through to the
/// system heuristic rather than risk a wrong sibling.
const FUZZY_ANCHOR_DF: u32 = 4;

/// Operator / utility words that appear in engine *affiliation*
/// segments (`RayariHydro_…`) and happen to also be rare words in some
/// unrelated wiki name. Excluded from anchor eligibility so a
/// coincidental affiliation-word overlap can't carry a match — e.g.
/// `hydro` must never bind `RayariHydro_McGarth` to `Terra Mills
/// HydroFarm`. They still contribute to *scoring* once a real anchor
/// exists; they just can't be the anchor themselves.
const AFFILIATION_NOISE: &[&str] = &[
    "hydro",
    "dynamics",
    "corp",
    "corporation",
    "industries",
    "industrial",
    "manufacturing",
    "security",
    "logistics",
    "aerospace",
];

impl LocationCatalog {
    /// Build the catalogue + all three indices in a single pass.
    /// Collisions on any index are resolved last-write-wins; the
    /// caller is expected to feed deterministic input (the
    /// upstream cron does — primary rows are unique on slug, and
    /// the enrichment cron is UPDATE-only).
    pub fn from_entries(entries: Vec<LocationCatalogEntry>) -> Self {
        let mut catalog = LocationCatalog::default();
        catalog.entries.reserve(entries.len());
        catalog.by_engine_tag.reserve(entries.len());
        catalog.by_slug.reserve(entries.len());
        catalog.by_normalized_name.reserve(entries.len());

        for entry in entries {
            if !entry.is_present() {
                continue;
            }
            let arc = Arc::new(entry);

            if let Some(tag) = arc.engine_tag.as_deref() {
                catalog
                    .by_engine_tag
                    .insert(tag.to_ascii_lowercase(), arc.clone());
            }
            catalog
                .by_slug
                .insert(arc.slug.to_ascii_lowercase(), arc.clone());
            catalog
                .by_normalized_name
                .insert(normalize_name(&arc.display_name), arc.clone());

            // Content tokens of the display name feed the fuzzy
            // matcher. Dedup per-entry so a name like "Pyro2 M Trdp 01"
            // counts each token once toward document frequency.
            let mut toks = content_tokens(&arc.display_name);
            toks.sort();
            toks.dedup();
            for tok in &toks {
                *catalog.name_token_df.entry(tok.clone()).or_insert(0) += 1;
            }
            catalog.entry_tokens.push(toks);
            catalog.entries.push(arc);
        }

        // Second pass: build the inverted index now that every token's
        // document frequency is known, skipping filler tokens.
        for (idx, toks) in catalog.entry_tokens.iter().enumerate() {
            for tok in toks {
                if catalog.name_token_df.get(tok).copied().unwrap_or(0) <= MAX_INDEX_DF {
                    catalog
                        .name_token_index
                        .entry(tok.clone())
                        .or_default()
                        .push(idx);
                }
            }
        }

        catalog
    }

    /// Fuzzy fallback used by the classifier when no exact engine-tag,
    /// slug, or normalized-name key matched. Scores catalog entries by
    /// inverse-document-frequency-weighted token overlap against the
    /// query tokens (already split on `_` and `<System><index>` by the
    /// classifier), and returns the single best entry — or `None` when
    /// no candidate clears the precision bar.
    ///
    /// Two guards keep precision high:
    ///   * **Distinctive-token requirement** — the winner must share a
    ///     non-digit token with df ≤ [`FUZZY_ANCHOR_DF`]. A match resting only
    ///     on filler (`"research"`, `"outpost"`) is rejected.
    ///   * **System consistency** — when the caller knows the system
    ///     (parsed from the engine string) and a candidate declares a
    ///     *different* system, that candidate is discarded. A
    ///     `Stanton…` engine string can never resolve to a Pyro row.
    ///
    /// Deterministic: ties break by score, then shared-token count,
    /// then slug — never by `HashMap` iteration order.
    pub fn fuzzy_match(
        &self,
        query_tokens: &[String],
        system_hint: Option<&str>,
    ) -> Option<&LocationCatalogEntry> {
        // Expand the query into its content-token set.
        let mut query: Vec<String> = Vec::new();
        for t in query_tokens {
            for tok in content_tokens(t) {
                if !query.contains(&tok) {
                    query.push(tok);
                }
            }
        }
        if query.is_empty() {
            return None;
        }

        // Gather candidate entries: any entry sharing a non-filler
        // token with the query.
        let mut candidates: Vec<usize> = Vec::new();
        for tok in &query {
            if let Some(idxs) = self.name_token_index.get(tok) {
                candidates.extend_from_slice(idxs);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();

        // Score each candidate over the FULL shared-token set (including
        // common tokens, weighted near-zero), and apply both guards.
        struct Scored {
            idx: usize,
            score: f32,
            shared: u32,
        }
        let mut best: Option<Scored> = None;
        for &i in &candidates {
            let entry = &self.entries[i];
            if let (Some(hint), Some(sys)) = (system_hint, entry.system.as_deref()) {
                if !hint.eq_ignore_ascii_case(sys) {
                    continue;
                }
            }
            let mut score = 0.0f32;
            let mut shared = 0u32;
            let mut has_anchor = false;
            for tok in &self.entry_tokens[i] {
                if !query.contains(tok) {
                    continue;
                }
                let df = self.name_token_df.get(tok).copied().unwrap_or(1).max(1);
                // Every shared token counts toward the ranking score
                // (so a shared digit still breaks SAL-2 from SAL-5)…
                score += 1.0 / df as f32;
                shared += 1;
                // …but only a rare, non-digit, non-affiliation token
                // qualifies as the *anchor* that licenses the match.
                if df <= FUZZY_ANCHOR_DF
                    && tok.len() >= 3
                    && !tok.chars().all(|c| c.is_ascii_digit())
                    && !AFFILIATION_NOISE.contains(&tok.as_str())
                {
                    has_anchor = true;
                }
            }
            if !has_anchor {
                continue;
            }
            let better = match &best {
                None => true,
                Some(b) => {
                    score > b.score
                        || (score == b.score && shared > b.shared)
                        || (score == b.score
                            && shared == b.shared
                            && entry.slug < self.entries[b.idx].slug)
                }
            };
            if better {
                best = Some(Scored {
                    idx: i,
                    score,
                    shared,
                });
            }
        }

        best.map(|b| self.entries[b.idx].as_ref())
    }

    /// Number of entries in the catalogue (post-dedup of empty rows).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up by the engine tag (`tag.name`). The strongest
    /// match — when the game log contains a string that decomposes
    /// to one of these tokens, this is the right binding.
    pub fn lookup_by_engine_tag(&self, tag: &str) -> Option<&LocationCatalogEntry> {
        self.by_engine_tag
            .get(&tag.to_ascii_lowercase())
            .map(|a| a.as_ref())
    }

    /// Look up by slug (`metadata.slug`).
    pub fn lookup_by_slug(&self, slug: &str) -> Option<&LocationCatalogEntry> {
        self.by_slug
            .get(&slug.to_ascii_lowercase())
            .map(|a| a.as_ref())
    }

    /// Look up by display name (case- and space-insensitive).
    pub fn lookup_by_name(&self, name: &str) -> Option<&LocationCatalogEntry> {
        self.by_normalized_name
            .get(&normalize_name(name))
            .map(|a| a.as_ref())
    }

    /// Walk the three indices in priority order: engine_tag →
    /// slug → normalized name. The first hit wins. Returns `None`
    /// when no index matches.
    pub fn lookup_by_token(&self, token: &str) -> Option<&LocationCatalogEntry> {
        self.lookup_by_engine_tag(token)
            .or_else(|| self.lookup_by_slug(token))
            .or_else(|| self.lookup_by_name(token))
    }

    /// Iterate every entry. Order is insertion-stable, which equals
    /// `from_entries` input order (modulo empty-row drops).
    pub fn iter(&self) -> impl Iterator<Item = &LocationCatalogEntry> + '_ {
        self.entries.iter().map(|a| a.as_ref())
    }
}

/// Lowercase + strip every non-alphanumeric char. Used to make
/// `"New Babbage"` and `"NewBabbage"` and `"new-babbage"` all key
/// the same row.
fn normalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

/// System names are dropped from content tokens — the classifier
/// tracks the system separately, and including it would let any two
/// same-system locations share a (useless) token.
const SYSTEM_TOKENS: &[&str] = &["stanton", "pyro", "nyx", "castra", "terra", "sol"];

/// Split a name or engine identifier into lowercase content tokens for
/// fuzzy matching. Boundaries: non-alphanumerics, camelCase humps, and
/// letter↔digit transitions. Drops system names and sub-3-char
/// non-numeric noise (single letters like a stray `b` from `1b`).
///
///   * `"RayariHydro_Deltana"`      → `["rayari", "hydro", "deltana"]`
///   * `"Shubin Mining SAL-2"`      → `["shubin", "mining", "sal", "2"]`
///   * `"Stanton4a_Shubin_SM0_13"`  → `["4a"→…, "shubin", "sm", "0", "13"]`
pub fn content_tokens(s: &str) -> Vec<String> {
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Upper,
        Lower,
        Digit,
        Other,
    }
    fn kind(c: char) -> Kind {
        if c.is_ascii_digit() {
            Kind::Digit
        } else if c.is_ascii_uppercase() {
            Kind::Upper
        } else if c.is_ascii_lowercase() {
            Kind::Lower
        } else {
            Kind::Other
        }
    }

    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<Kind> = None;
    for c in s.chars() {
        let k = kind(c);
        let boundary = matches!(
            (prev, k),
            (_, Kind::Other)
                | (Some(Kind::Other), _)
                | (Some(Kind::Lower), Kind::Upper)
                | (Some(Kind::Digit), Kind::Upper)
                | (Some(Kind::Lower), Kind::Digit)
                | (Some(Kind::Upper), Kind::Digit)
                | (Some(Kind::Digit), Kind::Lower)
        );
        if boundary && !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
        if k != Kind::Other {
            cur.push(c.to_ascii_lowercase());
        }
        prev = Some(k);
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }

    tokens.retain(|t| {
        let all_digit = t.chars().all(|c| c.is_ascii_digit());
        (t.len() >= 3 || all_digit) && !SYSTEM_TOKENS.contains(&t.as_str())
    });
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location_taxonomy::{LocationTier, Placement};

    fn entry(slug: &str, name: &str, class_name: &str, tag: Option<&str>) -> LocationCatalogEntry {
        LocationCatalogEntry {
            slug: slug.into(),
            display_name: name.into(),
            class_name: class_name.into(),
            engine_tag: tag.map(str::to_string),
            system: None,
            parent_body: None,
            classification: None,
            taxonomy: LocationTaxonomy::default(),
        }
    }

    #[test]
    fn build_indexes_every_entry_under_three_keys() {
        let cat = LocationCatalog::from_entries(vec![
            entry("aberdeen", "Aberdeen", "Aberdeen", Some("Stanton1b")),
            entry("lorville", "Lorville", "Lorville", Some("Lorville")),
            entry(
                "new-babbage",
                "New Babbage",
                "NewBabbage",
                Some("MicroTech_Babbage"),
            ),
        ]);
        assert_eq!(cat.len(), 3);

        // Engine tag hit (case-insensitive).
        assert_eq!(
            cat.lookup_by_engine_tag("STANTON1B").unwrap().display_name,
            "Aberdeen"
        );

        // Slug hit (case-insensitive).
        assert_eq!(
            cat.lookup_by_slug("LORVILLE").unwrap().display_name,
            "Lorville"
        );

        // Display-name hit (case-+space-insensitive).
        assert_eq!(
            cat.lookup_by_name("newbabbage").unwrap().slug,
            "new-babbage"
        );
        assert_eq!(
            cat.lookup_by_name("New Babbage").unwrap().slug,
            "new-babbage"
        );
        assert_eq!(
            cat.lookup_by_name("new-babbage").unwrap().slug,
            "new-babbage"
        );
    }

    #[test]
    fn lookup_by_token_walks_indexes_in_priority_order() {
        let cat = LocationCatalog::from_entries(vec![entry(
            "aberdeen",
            "Aberdeen",
            "Aberdeen",
            Some("Stanton1b"),
        )]);
        // Engine tag wins when matched.
        assert!(cat.lookup_by_token("Stanton1b").is_some());
        // Slug fallback hits.
        assert!(cat.lookup_by_token("aberdeen").is_some());
        // Name fallback hits.
        assert!(cat.lookup_by_token("Aberdeen").is_some());
        // Misses are None.
        assert!(cat.lookup_by_token("Pyro").is_none());
    }

    #[test]
    fn empty_rows_are_dropped_silently() {
        let cat = LocationCatalog::from_entries(vec![
            LocationCatalogEntry {
                slug: String::new(),
                display_name: "ghost".into(),
                class_name: String::new(),
                ..Default::default()
            },
            entry("real", "Real", "Real", None),
        ]);
        assert_eq!(cat.len(), 1);
    }

    #[test]
    fn entries_without_engine_tag_still_index_by_slug_and_name() {
        let cat = LocationCatalog::from_entries(vec![entry("endgame", "Endgame", "Endgame", None)]);
        assert!(cat.lookup_by_engine_tag("Endgame").is_none());
        assert!(cat.lookup_by_slug("endgame").is_some());
        assert!(cat.lookup_by_name("endgame").is_some());
    }

    #[test]
    fn taxonomy_round_trips_through_the_catalog() {
        let mut e = entry("jumptown", "Jumptown", "Jumptown", Some("Jumptown"));
        e.taxonomy.tier = Some(LocationTier::Landmark);
        e.taxonomy.subtype = Some("drug_lab".into());
        e.taxonomy.placement = Some(Placement::OnBody {
            body: "Daymar".into(),
        });
        let cat = LocationCatalog::from_entries(vec![e]);
        let hit = cat.lookup_by_token("Jumptown").unwrap();
        assert_eq!(hit.taxonomy.tier, Some(LocationTier::Landmark));
        assert_eq!(hit.taxonomy.subtype.as_deref(), Some("drug_lab"));
    }
}
