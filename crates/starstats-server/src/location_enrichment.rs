//! Star Citizen location-taxonomy enrichment from
//! [`starcitizen.tools`](https://starcitizen.tools).
//!
//! Companion to `reference_data.rs`. Where `reference_data.rs` pulls
//! the structured primary catalogue from `api.star-citizen.wiki`
//! (1955 entries, engine join key, system/parent/tag), this module
//! pulls the **enrichment** layer from `starcitizen.tools` — the
//! richer 7-tier taxonomy (Landmarks/Naval bases/Flotillas + 23
//! Landmark sub-buckets) and the spatial-relation tags
//! (`On <Body>` / `Orbits <Body>` / `Lagrange Point Lx <Body>`).
//!
//! The two sources are joined at the store layer by slug — the
//! primary cron seeds rows, this enrichment cron UPDATES them
//! (never INSERTs). A starcitizen.tools page with no matching
//! primary entry is skipped + logged. See
//! `docs/PLAN-LOCATION-TAXONOMY-V2.md` for the surrounding plan and
//! `memory/sc-wiki-location-taxonomy.md` for the underlying source
//! taxonomy.
//!
//! ## API shape
//!
//! `starcitizen.tools` exposes a standard MediaWiki API at
//! `/api.php`. We use two endpoints:
//!
//!  - `action=query&list=categorymembers&cmtitle=Category:Locations`
//!    to enumerate all location page titles (paginated via
//!    `cmcontinue`, 500 per page).
//!  - `action=query&prop=categories&titles=A|B|C&cllimit=100`
//!    to fetch each page's category set in batches of 50 titles.
//!
//! ~1073 location pages → 3 enumeration requests + ~22 category
//! batches = ~25 requests per daily cron run. Comfortably under any
//! reasonable rate limit.
//!
//! ## Failure semantics
//!
//! Like `WikiReferenceClient`, failure modes deliberately collapse
//! to [`LocationEnrichmentOutcome::UpstreamUnavailable`]. The caller
//! logs and falls back to whatever enrichment is already in the
//! store — stale taxonomy is better than no taxonomy.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use starstats_core::location_taxonomy::{
    parse_categories_to_taxonomy, slug_from_page_title, LocationTaxonomy,
};

/// HTTP timeout per request. Slightly more generous than the
/// vehicle/weapon cron (10s) because `starcitizen.tools` is a
/// community-hosted MediaWiki and can be slower under load.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Max titles per `prop=categories` batch. MediaWiki accepts up to
/// 50 per call without a `apihighlimits` user right.
const TITLE_BATCH_SIZE: usize = 50;

/// Category-members page size. MediaWiki caps at 500 for anonymous
/// callers, which is what we are.
const CMLIMIT: u32 = 500;

/// Per-page categories. The richest pages we've seen carry ~15
/// categories; 100 gives 6x headroom.
const CLLIMIT: u32 = 100;

/// Hard cap on enumeration round-trips. 1073 / 500 = 3 today; cap of
/// 20 gives ample headroom for upstream growth before the wiki adds
/// ~10x more location pages — and a circuit-breaker if the
/// `cmcontinue` chain ever loops.
const MAX_ENUMERATION_REQUESTS: u32 = 20;

/// Hard cap on category-fetch round-trips. 1073 / 50 = 22 today;
/// cap of 200 leaves room for 10x growth.
const MAX_CATEGORY_BATCH_REQUESTS: u32 = 200;

/// Per-response body cap. MediaWiki API responses are JSON; even a
/// 500-page categorymembers response runs ~50 KB. 4 MB is 80x that.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

const USER_AGENT: &str = concat!(
    "StarStats/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/TheCodeSaiyan/StarStats-Platform; location taxonomy enrichment)"
);

const TOOLS_API_BASE: &str = "https://starcitizen.tools/api.php";
const ROOT_CATEGORY: &str = "Category:Locations";

/// Outcome of an enrichment fetch. The `Entries` variant carries a
/// `HashMap<slug, LocationTaxonomy>` keyed on the slug form of the
/// page title (see [`slug_from_page_title`]) so the store can
/// `UPDATE … WHERE category='location' AND lower(slug) = lower($n)`
/// without further normalization.
///
/// `serde_json::Value` doesn't impl `Eq`, so this enum is PartialEq
/// only — mirroring `ReferenceFetchOutcomeCategory`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationEnrichmentOutcome {
    Entries(HashMap<String, LocationTaxonomy>),
    UpstreamUnavailable,
}

#[async_trait]
pub trait LocationEnrichmentClient: Send + Sync + 'static {
    /// Fetch the full taxonomy enrichment set. Implementations are
    /// expected to handle pagination internally and return all known
    /// (slug → taxonomy) pairs as a single map.
    async fn fetch_all(&self) -> LocationEnrichmentOutcome;
}

/// Production [`LocationEnrichmentClient`] backed by `reqwest`.
pub struct ToolsWikiEnrichmentClient {
    inner: reqwest::Client,
    api_base: String,
}

impl ToolsWikiEnrichmentClient {
    /// Construct against the production `starcitizen.tools` host.
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::with_base(TOOLS_API_BASE.to_string())
    }

    /// Construct against an arbitrary MediaWiki API base. Used by
    /// tests to point at a `mockito` server.
    pub fn with_base(api_base: String) -> Result<Self, reqwest::Error> {
        let inner = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()?;
        Ok(Self { inner, api_base })
    }
}

#[async_trait]
impl LocationEnrichmentClient for ToolsWikiEnrichmentClient {
    async fn fetch_all(&self) -> LocationEnrichmentOutcome {
        let titles = match enumerate_locations(&self.inner, &self.api_base).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "location enrichment: enumeration failed");
                return LocationEnrichmentOutcome::UpstreamUnavailable;
            }
        };

        if titles.is_empty() {
            tracing::warn!("location enrichment: enumeration returned 0 titles (upstream empty?)");
            return LocationEnrichmentOutcome::UpstreamUnavailable;
        }

        let mut out: HashMap<String, LocationTaxonomy> = HashMap::with_capacity(titles.len());

        for (batch_idx, batch) in titles.chunks(TITLE_BATCH_SIZE).enumerate() {
            if batch_idx as u32 >= MAX_CATEGORY_BATCH_REQUESTS {
                tracing::warn!("location enrichment: hit max category-batch requests; truncating");
                break;
            }

            match fetch_category_batch(&self.inner, &self.api_base, batch).await {
                Ok(pages) => {
                    for (title, cats) in pages {
                        let slug = slug_from_page_title(&title);
                        if slug.is_empty() {
                            continue;
                        }
                        let taxonomy = parse_categories_to_taxonomy(&cats);
                        if !taxonomy.is_empty() {
                            // Last-write-wins on slug collisions
                            // (e.g. two pages with the same
                            // disambig-stripped slug). Vanishingly
                            // rare on real wiki data; the store
                            // layer's slug uniqueness invariant
                            // catches it for forensics.
                            out.insert(slug, taxonomy);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        batch_size = batch.len(),
                        "location enrichment: category batch failed; continuing"
                    );
                    // Don't abort — partial enrichment is more
                    // useful than none. The next daily cron tries
                    // again.
                }
            }
        }

        tracing::info!(
            titles_seen = titles.len(),
            entries_produced = out.len(),
            "location enrichment: fetch complete"
        );

        LocationEnrichmentOutcome::Entries(out)
    }
}

// ---- enumeration --------------------------------------------------

/// Walk `Category:Locations` via `categorymembers` pagination and
/// collect every page title. Boilerplate categories (e.g. `Pages
/// needing citations`) live in their own namespace and don't appear
/// in our `cmtype=page` results, so no filtering is needed here.
async fn enumerate_locations(
    client: &reqwest::Client,
    api_base: &str,
) -> Result<Vec<String>, EnrichmentError> {
    let mut titles = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    let mut cm_continue: Option<String> = None;
    let mut requests = 0u32;

    loop {
        if requests >= MAX_ENUMERATION_REQUESTS {
            tracing::warn!("location enrichment: hit max enumeration requests; truncating");
            break;
        }
        requests += 1;

        let mut url = format!(
            "{api_base}?action=query&list=categorymembers&cmtitle={cat}&cmtype=page&cmlimit={lim}&format=json",
            cat = urlencoded(ROOT_CATEGORY),
            lim = CMLIMIT
        );
        if let Some(ref c) = cm_continue {
            url.push_str("&cmcontinue=");
            url.push_str(&urlencoded(c));
        }

        let body = fetch_json(client, &url).await?;

        let resp: CategoryMembersResponse =
            serde_json::from_str(&body).map_err(|e| EnrichmentError::ParseJson(e.to_string()))?;

        for m in resp.query.categorymembers {
            if seen.insert(m.title.clone()) {
                titles.push(m.title);
            }
        }

        match resp.continue_block {
            Some(c) if !c.cmcontinue.is_empty() => cm_continue = Some(c.cmcontinue),
            _ => break,
        }
    }

    Ok(titles)
}

#[derive(Debug, Deserialize)]
struct CategoryMembersResponse {
    query: CategoryMembersQuery,
    #[serde(default, rename = "continue")]
    continue_block: Option<ContinueBlock>,
}

#[derive(Debug, Deserialize)]
struct CategoryMembersQuery {
    categorymembers: Vec<CategoryMember>,
}

#[derive(Debug, Deserialize)]
struct CategoryMember {
    title: String,
}

#[derive(Debug, Deserialize)]
struct ContinueBlock {
    #[serde(default)]
    cmcontinue: String,
}

// ---- category batch fetch ----------------------------------------

/// Fetch the category list for up to 50 titles in a single
/// `prop=categories` round-trip. Returns `(title → Vec<category_name>)`
/// with the `Category:` prefix already stripped.
async fn fetch_category_batch(
    client: &reqwest::Client,
    api_base: &str,
    titles: &[String],
) -> Result<HashMap<String, Vec<String>>, EnrichmentError> {
    let joined = titles.join("|");
    let url = format!(
        "{api_base}?action=query&prop=categories&titles={titles}&cllimit={lim}&format=json",
        titles = urlencoded(&joined),
        lim = CLLIMIT
    );

    let body = fetch_json(client, &url).await?;
    parse_category_batch_body(&body)
}

/// Pure parser for the `prop=categories` response. Extracted so the
/// test suite can pin response-shape parsing without standing up an
/// HTTP server.
fn parse_category_batch_body(body: &str) -> Result<HashMap<String, Vec<String>>, EnrichmentError> {
    let parsed: PropCategoriesResponse =
        serde_json::from_str(body).map_err(|e| EnrichmentError::ParseJson(e.to_string()))?;

    let mut out: HashMap<String, Vec<String>> = HashMap::new();

    for (_id, page) in parsed.query.pages {
        // Pages whose title was misspelled by the caller come back
        // with `missing: true`; skip those silently — the
        // `Klescher Rehabilitation Facility` case is the canonical
        // example.
        if page.missing.unwrap_or(false) {
            continue;
        }
        let cats = page
            .categories
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| {
                let stripped = c.title.strip_prefix("Category:")?;
                // Filter out MediaWiki boilerplate categories that
                // describe the wiki software's state, not the
                // location's lore (e.g. `Pages using DynamicPageList4`,
                // `Pages with outdated information`). The parser
                // tolerates them via `additional_categories`, but
                // dropping them at ingest time keeps the bounded
                // 32-slot bucket from filling with noise.
                if stripped.starts_with("Pages ") || stripped.starts_with("Articles ") {
                    return None;
                }
                Some(stripped.to_string())
            })
            .collect::<Vec<String>>();
        out.insert(page.title, cats);
    }

    Ok(out)
}

#[derive(Debug, Deserialize)]
struct PropCategoriesResponse {
    query: PropCategoriesQuery,
}

#[derive(Debug, Deserialize)]
struct PropCategoriesQuery {
    #[serde(default)]
    pages: HashMap<String, PropCategoriesPage>,
}

#[derive(Debug, Deserialize)]
struct PropCategoriesPage {
    title: String,
    #[serde(default)]
    categories: Option<Vec<PropCategoryEntry>>,
    #[serde(default)]
    missing: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PropCategoryEntry {
    title: String,
}

// ---- HTTP helpers ------------------------------------------------

async fn fetch_json(client: &reqwest::Client, url: &str) -> Result<String, EnrichmentError> {
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| EnrichmentError::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(EnrichmentError::Http(format!(
            "HTTP {} for {}",
            status, url
        )));
    }
    // Use bytes + size cap rather than .text() so a misbehaving
    // upstream can't balloon a single allocation. Mirrors the
    // primary client's MAX_PAGE_BODY_BYTES policy.
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| EnrichmentError::Http(e.to_string()))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(EnrichmentError::Http(format!(
            "response body {} bytes exceeds cap {}",
            bytes.len(),
            MAX_RESPONSE_BYTES
        )));
    }
    String::from_utf8(bytes.to_vec()).map_err(|e| EnrichmentError::ParseJson(e.to_string()))
}

fn urlencoded(s: &str) -> String {
    // Minimal percent-encoding for the characters we actually emit
    // into query strings: space, pipe, colon, and the few that
    // appear in wiki titles. Stays dependency-free.
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for &b in ch.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum EnrichmentError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("JSON parse error: {0}")]
    ParseJson(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_category_batch_body ---------------------------------

    #[test]
    fn parse_batch_body_extracts_titles_and_categories() {
        // Real MediaWiki API response shape, abridged. Two pages —
        // one with categories, one marked `missing` (the
        // Klescher case).
        let body = r#"{
            "query": {
                "pages": {
                    "12345": {
                        "title": "Lorville",
                        "categories": [
                            {"title": "Category:Cities"},
                            {"title": "Category:Landing zones"},
                            {"title": "Category:Locations"},
                            {"title": "Category:On Hurston"},
                            {"title": "Category:Pages using DynamicPageList4"},
                            {"title": "Category:Stanton system"}
                        ]
                    },
                    "-1": {
                        "title": "Klescher Rehabilitation Facility",
                        "missing": true
                    }
                }
            }
        }"#;

        let result = parse_category_batch_body(body).expect("parse ok");

        // The missing page is dropped.
        assert!(!result.contains_key("Klescher Rehabilitation Facility"));

        // Boilerplate `Pages …` category is filtered out; the rest
        // come through with the `Category:` prefix stripped.
        let cats = result.get("Lorville").expect("lorville present");
        assert_eq!(
            cats,
            &vec![
                "Cities".to_string(),
                "Landing zones".to_string(),
                "Locations".to_string(),
                "On Hurston".to_string(),
                "Stanton system".to_string(),
            ]
        );
    }

    #[test]
    fn parse_batch_body_handles_empty_pages_block() {
        let body = r#"{ "query": { "pages": {} } }"#;
        let r = parse_category_batch_body(body).expect("parse ok");
        assert!(r.is_empty());
    }

    #[test]
    fn parse_batch_body_handles_page_without_categories_field() {
        // MediaWiki omits `categories` entirely when a page has none.
        let body = r#"{
            "query": {
                "pages": {
                    "999": {
                        "title": "Orphan Page"
                    }
                }
            }
        }"#;
        let r = parse_category_batch_body(body).expect("parse ok");
        let cats = r.get("Orphan Page").expect("present");
        assert!(cats.is_empty());
    }

    #[test]
    fn parse_batch_body_rejects_malformed_json() {
        let body = "not json at all";
        let r = parse_category_batch_body(body);
        assert!(matches!(r, Err(EnrichmentError::ParseJson(_))));
    }

    // ---- urlencoded ------------------------------------------------

    #[test]
    fn urlencoded_passes_safe_chars_through() {
        assert_eq!(urlencoded("Hurston"), "Hurston");
        assert_eq!(urlencoded("HUR-L1"), "HUR-L1");
        assert_eq!(urlencoded("hello_world.txt~"), "hello_world.txt~");
    }

    #[test]
    fn urlencoded_encodes_titles_with_spaces_and_pipes() {
        assert_eq!(
            urlencoded("Lorville|Area18|Port Olisar"),
            "Lorville%7CArea18%7CPort%20Olisar"
        );
    }

    #[test]
    fn urlencoded_handles_apostrophes_and_parens() {
        // From real wiki titles: `Rod's Fuel 'N Supplies`,
        // `Klescher Rehabilitation Facility (Aberdeen)`.
        assert_eq!(urlencoded("Rod's Fuel"), "Rod%27s%20Fuel");
        assert_eq!(urlencoded("Foo (Bar)"), "Foo%20%28Bar%29");
    }

    #[test]
    fn urlencoded_encodes_non_ascii_as_utf8_bytes() {
        // `-60° from Monox` round-trip — the degree sign is two
        // UTF-8 bytes (0xC2 0xB0).
        assert_eq!(urlencoded("60\u{00B0}"), "60%C2%B0");
    }

    // ---- Test stub for trait wiring (Phase 1.4/1.5 use it) --------

    struct StubClient {
        outcome: LocationEnrichmentOutcome,
    }

    #[async_trait]
    impl LocationEnrichmentClient for StubClient {
        async fn fetch_all(&self) -> LocationEnrichmentOutcome {
            self.outcome.clone()
        }
    }

    #[tokio::test]
    async fn stub_client_returns_unavailable_when_configured() {
        let c = StubClient {
            outcome: LocationEnrichmentOutcome::UpstreamUnavailable,
        };
        assert_eq!(
            c.fetch_all().await,
            LocationEnrichmentOutcome::UpstreamUnavailable
        );
    }

    #[tokio::test]
    async fn stub_client_returns_entries_when_configured() {
        let mut entries = HashMap::new();
        entries.insert("lorville".to_string(), LocationTaxonomy::default());
        let c = StubClient {
            outcome: LocationEnrichmentOutcome::Entries(entries.clone()),
        };
        assert_eq!(
            c.fetch_all().await,
            LocationEnrichmentOutcome::Entries(entries)
        );
    }
}
