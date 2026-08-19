//! Star Citizen vehicle reference data fetched from the Wiki API.
//!
//! Game events store internal class names like
//! `AEGS_Avenger_Stalker_Living` — the dashboard wants to render
//! "Aegis Avenger Stalker" instead. The [`ReferenceClient`] trait
//! fronts the upstream lookup so the daily refresh job can be
//! tested without hitting the network. Production
//! [`WikiReferenceClient`] paginates through
//! `https://api.star-citizen.wiki/api/v3/vehicles` and returns the
//! full vehicle catalogue as a single `Vec`.
//!
//! Failure modes deliberately collapse to
//! [`ReferenceFetchOutcome::UpstreamUnavailable`]: the caller logs
//! and falls back to whatever's already in the store. There is no
//! fine-grained error taxonomy because the only thing the caller
//! needs to know is "did we get fresh data, or are we still on the
//! stale cache." The trade-off mirrors `rsi_verify::HttpRsiClient`.
//!
//! Per-vehicle JSON parsing lives in the pure [`parse_vehicles_page`]
//! function so the test suite can exercise it without standing up a
//! mock HTTP server. The HTTP layer is a thin shell around
//! `reqwest` + this parser.

use async_trait::async_trait;
use std::time::Duration;

/// One vehicle pulled from the Wiki API. Field shape matches the
/// `vehicle_reference` table (see `migrations/0012_reference_data.sql`).
///
/// `class_name` is the internal Star Citizen class identifier and the
/// join key against event payloads. It's case-sensitive on the way
/// in; the store performs case-insensitive lookups via the
/// `lower(class_name)` index since game logs occasionally vary case.
///
/// All metadata fields except `display_name` are `Option`: the Wiki
/// API returns inconsistent shapes per vehicle and we'd rather store
/// `None` than synthesise a value the upstream didn't actually
/// publish. Empty / whitespace-only strings collapse to `None` at
/// parse time so the storage layer never sees `Some("")`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct VehicleReference {
    /// Internal game class name (e.g. "AEGS_Avenger_Stalker"). Used as
    /// the join key against event payloads. Case-sensitive on the way
    /// in, but the store lookups are case-insensitive (lower() index).
    pub class_name: String,
    /// Player-friendly name from the Wiki ("Aegis Avenger Stalker").
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull_size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

/// Result of a fetch against the upstream Wiki. Two outcomes:
/// either we got the full catalogue (possibly empty), or the upstream
/// is unavailable / misbehaving and the caller should keep serving
/// whatever's already cached. There is deliberately no
/// "partial success" variant — a half-paginated walk is worse than
/// no refresh at all because it would corrupt the cache by deleting
/// vehicles that simply hadn't been fetched yet (if the caller
/// implements a "delete missing" policy in a future slice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceFetchOutcome {
    Vehicles(Vec<VehicleReference>),
    UpstreamUnavailable,
}

/// Top-level category an entry in the generic `reference_registry`
/// belongs to. Mirrors the `reference_registry_category_chk` CHECK
/// constraint in migration 0022 — adding a category requires a
/// follow-up migration to widen the allow-list.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceCategory {
    Vehicle,
    Weapon,
    Item,
    Location,
}

// `as_str` / `parse` are wired in by the store refactor (P2) and
// route layer (P4) — silence dead-code during the transition.
#[allow(dead_code)]
impl ReferenceCategory {
    /// Lowercase string form — the value stored in the `category`
    /// column and used in the public route segment.
    pub fn as_str(self) -> &'static str {
        match self {
            ReferenceCategory::Vehicle => "vehicle",
            ReferenceCategory::Weapon => "weapon",
            ReferenceCategory::Item => "item",
            ReferenceCategory::Location => "location",
        }
    }

    /// Parse from the route segment. Returns `None` on any value
    /// outside the CHECK-constraint allow-list so route handlers can
    /// 404 unknown categories rather than letting them reach the DB.
    pub fn parse(s: &str) -> Option<ReferenceCategory> {
        match s {
            "vehicle" => Some(ReferenceCategory::Vehicle),
            "weapon" => Some(ReferenceCategory::Weapon),
            "item" => Some(ReferenceCategory::Item),
            "location" => Some(ReferenceCategory::Location),
            _ => None,
        }
    }
}

/// A single entry in the generic reference registry. Per-category
/// extras live in `metadata` as a JSON object — schema-on-read — so
/// new categories can ship without DDL. `VehicleReference` (above)
/// remains the typed view callers use for vehicle-specific rendering;
/// once the store refactor lands it will be decoded from a
/// `ReferenceEntry` with `category == Vehicle`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ReferenceEntry {
    pub category: ReferenceCategory,
    pub class_name: String,
    pub display_name: String,
    /// URL-safe canonical identifier, lowercased ASCII alphanumeric +
    /// hyphens. Derived from `display_name` at sync time (or from the
    /// wiki's own slug for locations). Collisions within a single
    /// category are resolved by appending `-2`, `-3`… to the later
    /// entries when ordered by `class_name`. Null on rows persisted
    /// before the KB-v1 migration; the route layer falls back to
    /// `class_name` lookup in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// JSON object holding per-category extras (manufacturer, role,
    /// size, slot, parent system…). `Default::default()` returns the
    /// empty object so unrenderable fields don't appear at all in
    /// JSON output. `serde_json::Value` does not implement `Eq`
    /// because of `f64`, so `ReferenceEntry` is `PartialEq` only.
    #[schema(value_type = Object)]
    #[serde(default, skip_serializing_if = "is_empty_object")]
    pub metadata: serde_json::Value,
}

fn is_empty_object(v: &serde_json::Value) -> bool {
    matches!(v, serde_json::Value::Object(m) if m.is_empty())
}

#[async_trait]
pub trait ReferenceClient: Send + Sync + 'static {
    /// Fetch the full vehicle reference set. Implementations are
    /// expected to paginate internally and return the full list as a
    /// single Vec. Failure modes collapse to UpstreamUnavailable; the
    /// caller logs and falls back to whatever's already in the store.
    ///
    /// Kept for backwards compatibility — new callers should prefer
    /// `fetch_category(ReferenceCategory::Vehicle)`. Unused by the
    /// in-tree cron after P3; the allow-dead silences the warning
    /// while the API stays available for external implementers.
    #[allow(dead_code)]
    async fn fetch_vehicles(&self) -> ReferenceFetchOutcome;

    /// Fetch the full catalogue for a single category. Returns
    /// generic `ReferenceEntry` items so callers can dispatch to one
    /// `upsert_entries` call regardless of category. Default impl
    /// reports the upstream as unavailable; the production
    /// `WikiReferenceClient` overrides it.
    async fn fetch_category(&self, _category: ReferenceCategory) -> ReferenceFetchOutcomeCategory {
        ReferenceFetchOutcomeCategory::UpstreamUnavailable
    }
}

/// Result of a generic category fetch. Mirrors `ReferenceFetchOutcome`
/// but holds a `Vec<ReferenceEntry>` so the caller can write all four
/// categories through a single store method. `serde_json::Value`
/// doesn't implement `Eq`, so this enum is `PartialEq` only.
#[derive(Debug, Clone, PartialEq)]
pub enum ReferenceFetchOutcomeCategory {
    Entries(Vec<ReferenceEntry>),
    UpstreamUnavailable,
}

// The wiki's documented API base is `/api/...`, not `/api/v3/...`. The
// v3-prefixed vehicles route works as a legacy alias but the other
// categories don't have one — P3's first attempt 404'd on weapons /
// items / locations because of this. The OpenAPI spec at
// `/api/openapi` lists every endpoint without the v3 prefix.
const WIKI_VEHICLES_BASE: &str = "https://api.star-citizen.wiki/api/vehicles";
const WIKI_WEAPONS_BASE: &str = "https://api.star-citizen.wiki/api/weapons";
const WIKI_ITEMS_BASE: &str = "https://api.star-citizen.wiki/api/items";
const WIKI_LOCATIONS_BASE: &str = "https://api.star-citizen.wiki/api/locations";

/// Page size we request from the wiki. The server caps at 200 even if
/// you ask for more, and the default is 30 — passing `?limit=200`
/// keeps the request count tractable on items (20k+ entries) without
/// the round-trip-per-30 fan-out the default produces.
const WIKI_PAGE_LIMIT: u32 = 200;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Hard cap on how many pages we'll walk. Sized for the biggest
/// known catalogue (items, ~20k entries → ~101 pages at limit=200)
/// with 2x headroom for upstream growth and the occasional "the API
/// is misbehaving / paginating endlessly" abort case.
const MAX_PAGE_REQUESTS: u32 = 250;
/// Per-page body cap. Vehicles at limit=200 currently runs ~4 MB per
/// page (rich per-vehicle metadata: components, specs, in-fiction
/// text); items / weapons / locations stay well under 1 MB. 16 MB
/// gives headroom for further upstream growth without letting a
/// misbehaving response balloon a single allocation.
const MAX_PAGE_BODY_BYTES: usize = 16 * 1024 * 1024;
/// Body cap across all pages combined. Items at limit=200 is
/// ~20 MB total; 200 MB leaves 10x headroom for upstream growth.
/// Enforced per-byte during streaming, not after `text()`
/// materialises the whole body.
const MAX_TOTAL_BODY_BYTES: usize = 200 * 1024 * 1024;
const USER_AGENT: &str = concat!(
    "StarStats/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/TheCodeSaiyan/StarStats-Platform)"
);

/// Production [`ReferenceClient`] backed by `reqwest`. Holds a shared
/// client so connection pooling + DNS caching survive across calls
/// (the daily refresh job invokes `fetch_vehicles` once, but tests
/// and ad-hoc admin tooling may spin a single instance up and reuse).
pub struct WikiReferenceClient {
    inner: reqwest::Client,
}

impl WikiReferenceClient {
    pub fn new() -> Result<Self, reqwest::Error> {
        let inner = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        Ok(Self { inner })
    }
}

#[async_trait]
impl ReferenceClient for WikiReferenceClient {
    async fn fetch_vehicles(&self) -> ReferenceFetchOutcome {
        let mut all = Vec::new();
        let mut page: u32 = 1;
        let mut total_bytes: usize = 0;

        loop {
            if page > MAX_PAGE_REQUESTS {
                tracing::warn!(
                    page,
                    cap = MAX_PAGE_REQUESTS,
                    "wiki vehicles paginated past safety cap; aborting"
                );
                return ReferenceFetchOutcome::UpstreamUnavailable;
            }

            let url = format!("{WIKI_VEHICLES_BASE}?page={page}&limit={WIKI_PAGE_LIMIT}");
            let resp = match self.inner.get(&url).send().await {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!(error = %err, page, "wiki vehicles fetch failed");
                    return ReferenceFetchOutcome::UpstreamUnavailable;
                }
            };

            let status = resp.status();
            if !status.is_success() {
                tracing::warn!(status = status.as_u16(), page, "wiki vehicles non-2xx");
                return ReferenceFetchOutcome::UpstreamUnavailable;
            }

            // Stream the body so we bail BEFORE allocating gigabytes
            // if the upstream misbehaves. `resp.text()` has no ceiling.
            let body =
                match read_capped_body(resp, ReferenceCategory::Vehicle, page, total_bytes).await {
                    Some(b) => b,
                    None => return ReferenceFetchOutcome::UpstreamUnavailable,
                };
            total_bytes = total_bytes.saturating_add(body.len());

            let json: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(err) => {
                    tracing::warn!(error = %err, page, "wiki vehicles json parse failed");
                    return ReferenceFetchOutcome::UpstreamUnavailable;
                }
            };

            all.extend(parse_vehicles_page(&json));

            // Pagination terminates when current_page reaches
            // last_page. Defensive: missing meta = single-page mode.
            let meta = json.get("meta");
            let current_page = meta
                .and_then(|m| m.get("current_page"))
                .and_then(|v| v.as_u64())
                .unwrap_or(page as u64);
            let last_page = meta
                .and_then(|m| m.get("last_page"))
                .and_then(|v| v.as_u64())
                .unwrap_or(current_page);

            if current_page >= last_page {
                break;
            }
            page += 1;
        }

        ReferenceFetchOutcome::Vehicles(all)
    }

    async fn fetch_category(&self, category: ReferenceCategory) -> ReferenceFetchOutcomeCategory {
        let base = match category {
            ReferenceCategory::Vehicle => WIKI_VEHICLES_BASE,
            ReferenceCategory::Weapon => WIKI_WEAPONS_BASE,
            ReferenceCategory::Item => WIKI_ITEMS_BASE,
            ReferenceCategory::Location => WIKI_LOCATIONS_BASE,
        };
        let mut all: Vec<ReferenceEntry> = Vec::new();
        let mut page: u32 = 1;
        let mut total_bytes: usize = 0;

        loop {
            if page > MAX_PAGE_REQUESTS {
                tracing::warn!(
                    category = category.as_str(),
                    page,
                    cap = MAX_PAGE_REQUESTS,
                    "wiki paginated past safety cap; aborting"
                );
                return ReferenceFetchOutcomeCategory::UpstreamUnavailable;
            }

            let url = format!("{base}?page={page}&limit={WIKI_PAGE_LIMIT}");
            let resp = match self.inner.get(&url).send().await {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!(error = %err, category = category.as_str(), page, "wiki fetch failed");
                    return ReferenceFetchOutcomeCategory::UpstreamUnavailable;
                }
            };

            let status = resp.status();
            if !status.is_success() {
                tracing::warn!(
                    status = status.as_u16(),
                    category = category.as_str(),
                    page,
                    "wiki non-2xx"
                );
                return ReferenceFetchOutcomeCategory::UpstreamUnavailable;
            }

            let body = match read_capped_body(resp, category, page, total_bytes).await {
                Some(b) => b,
                None => return ReferenceFetchOutcomeCategory::UpstreamUnavailable,
            };
            total_bytes = total_bytes.saturating_add(body.len());

            let json: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(err) => {
                    tracing::warn!(error = %err, category = category.as_str(), page, "wiki json parse failed");
                    return ReferenceFetchOutcomeCategory::UpstreamUnavailable;
                }
            };

            all.extend(parse_category_page(&json, category));

            let meta = json.get("meta");
            let current_page = meta
                .and_then(|m| m.get("current_page"))
                .and_then(|v| v.as_u64())
                .unwrap_or(page as u64);
            let last_page = meta
                .and_then(|m| m.get("last_page"))
                .and_then(|v| v.as_u64())
                .unwrap_or(current_page);

            if current_page >= last_page {
                break;
            }
            page += 1;
        }

        // Post-pass: assign slugs across the whole batch so collision
        // suffixes are stable across pages. Locations reuse the
        // wiki's `metadata.slug`; everything else derives from
        // `display_name`. Idempotent — safe to call twice.
        // Fold commodity container/form variants BEFORE slugs are
        // assigned, so the survivor keeps the clean slug rather than a
        // `-2` suffix inherited from a sibling that no longer exists.
        collapse_commodity_variants(&mut all);
        apply_slug_collisions(&mut all);

        ReferenceFetchOutcomeCategory::Entries(all)
    }
}

/// Stream a response body into a `Vec<u8>`, bailing out the moment it
/// crosses the per-page or cumulative cap. `reqwest::Response::text`
/// has no ceiling, so a misbehaving upstream could balloon a
/// server-side allocation. The cumulative limit is checked against the
/// running total carried across pages.
async fn read_capped_body(
    mut resp: reqwest::Response,
    category: ReferenceCategory,
    page: u32,
    bytes_so_far: usize,
) -> Option<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len().saturating_add(chunk.len()) > MAX_PAGE_BODY_BYTES {
                    tracing::warn!(
                        cap_bytes = MAX_PAGE_BODY_BYTES,
                        category = category.as_str(),
                        page,
                        "wiki per-page body exceeded cap; aborting"
                    );
                    return None;
                }
                if bytes_so_far
                    .saturating_add(buf.len())
                    .saturating_add(chunk.len())
                    > MAX_TOTAL_BODY_BYTES
                {
                    tracing::warn!(
                        cap_bytes = MAX_TOTAL_BODY_BYTES,
                        category = category.as_str(),
                        "wiki cumulative body exceeded cap; aborting"
                    );
                    return None;
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => return Some(buf),
            Err(err) => {
                tracing::warn!(error = %err, category = category.as_str(), page, "wiki body read failed");
                return None;
            }
        }
    }
}

/// Pull every well-formed vehicle out of a single Wiki API page.
///
/// Defensive on every field: the upstream JSON shape varies
/// per vehicle (some entries lack a manufacturer record, some have
/// `role` instead of `focus`, etc.) so we treat missing/null/empty
/// strings as `None` after trimming. The only hard requirement is a
/// non-empty `class_name` — without the join key the entry can't
/// link back to events, so it's dropped.
pub fn parse_vehicles_page(json: &serde_json::Value) -> Vec<VehicleReference> {
    let Some(data) = json.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    let mut out = Vec::with_capacity(data.len());
    for entry in data {
        // Drop the entry the moment we can't lift a usable join key.
        let Some(class_name) = string_field(entry, "class_name") else {
            continue;
        };

        // Display name falls back to the class name only as a last
        // resort — a player would rather see "AEGS_Avenger_Stalker"
        // than nothing at all if the upstream record is half-formed.
        let display_name = string_field(entry, "name").unwrap_or_else(|| class_name.clone());

        let manufacturer = entry.get("manufacturer").and_then(|m| {
            // Preferred shape: nested object with `name` / `code`.
            // Fall back to a flat string if the upstream simplified
            // the field on that vehicle.
            if m.is_object() {
                string_field(m, "name").or_else(|| string_field(m, "code"))
            } else if m.is_string() {
                m.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            } else {
                None
            }
        });

        // Wiki vehicles publish the same field as `focus` on most
        // records and `role` on a minority — check both before giving
        // up. `type` is a third sibling field but it's coarser
        // ("MultiCrew Combat") and we keep it out so the role column
        // doesn't get noisy.
        let role = string_field(entry, "role").or_else(|| string_field(entry, "focus"));
        let focus = string_field(entry, "focus");
        let hull_size = string_field(entry, "size");

        out.push(VehicleReference {
            class_name,
            display_name,
            manufacturer,
            role,
            hull_size,
            focus,
        });
    }
    out
}

/// Generic per-page parser for any category. Pulls each item's class
/// identifier (with fallbacks: class_name → code → slug → ref) and
/// display name, then collects every remaining top-level field into
/// the metadata JSONB blob. Internal Wiki bookkeeping fields (`id`,
/// `created_at`, `updated_at`, `version`) are stripped so they don't
/// pollute the catalogue with noise the dashboard can't use.
///
/// Defensive shape: an entry without a usable class identifier is
/// dropped silently — without the join key it can't link back to an
/// event payload, so storing it would be inert.
pub fn parse_category_page(
    json: &serde_json::Value,
    category: ReferenceCategory,
) -> Vec<ReferenceEntry> {
    let Some(data) = json.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(data.len());
    for entry in data {
        let Some(obj) = entry.as_object() else {
            continue;
        };

        let class_name = string_field(entry, "class_name")
            .or_else(|| string_field(entry, "code"))
            .or_else(|| string_field(entry, "slug"))
            .or_else(|| string_field(entry, "ref"));
        let Some(class_name) = class_name else {
            continue;
        };

        let display_name = string_field(entry, "name").unwrap_or_else(|| class_name.clone());

        let mut metadata = serde_json::Map::new();
        for (k, v) in obj.iter() {
            if matches!(
                k.as_str(),
                "class_name" | "name" | "id" | "created_at" | "updated_at" | "version"
            ) {
                continue;
            }
            metadata.insert(k.clone(), v.clone());
        }

        out.push(ReferenceEntry {
            category,
            class_name,
            display_name,
            // Slug is assigned in a post-pass (`apply_slug_collisions`)
            // after the whole batch is materialised — we need the
            // batch-wide view to resolve display-name collisions
            // deterministically. Leave None here so single-page parse
            // tests can assert the parser's output verbatim.
            slug: None,
            metadata: serde_json::Value::Object(metadata),
        });
    }
    out
}

/// Derive a URL-safe slug from a display name. Falls back to the
/// class name when display name is empty after slugification — and
/// to `unknown` as a last resort so the result is never empty.
///
/// Rules:
///   - lowercased ASCII alphanumeric only; everything else (spaces,
///     punctuation, non-ASCII letters) becomes a single hyphen.
///   - runs of hyphens collapse to one.
///   - leading/trailing hyphens trimmed.
///
/// Examples:
///   - `derive_slug("Aegis Avenger Stalker", "AEGS_Avenger_Stalker")`
///     → `"aegis-avenger-stalker"`
///   - `derive_slug("", "AEGS_Avenger_Stalker")`
///     → `"aegs-avenger-stalker"`
///   - `derive_slug("Klaus & Werner Sledge II", "KLWE_…")`
///     → `"klaus-werner-sledge-ii"`
pub fn derive_slug(display_name: &str, class_name: &str) -> String {
    let primary = slugify_ascii(display_name);
    if !primary.is_empty() {
        return primary;
    }
    let fallback = slugify_ascii(class_name);
    if !fallback.is_empty() {
        return fallback;
    }
    // Both sources slugified to empty — extremely defensive case
    // (would require both fields to be all-punctuation). Returning a
    // literal keeps callers safe from ever seeing `""`.
    "unknown".to_string()
}

/// ASCII-only slugifier. Returns lowercase a–z / 0–9 with hyphens
/// between word runs; leading / trailing / consecutive hyphens are
/// stripped. Non-ASCII letters are dropped (no transliteration) —
/// the wiki entries we care about are all-ASCII in practice and the
/// last-resort `unknown` fallback in [`derive_slug`] catches edge
/// cases like a wholly Cyrillic display name.
fn slugify_ascii(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_hyphen = true; // start true so leading hyphens are skipped
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            out.push('-');
            last_was_hyphen = true;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    out
}

/// Curated per-category projection of `metadata`. Powers the
/// browse-page chip filters and the entity hover-card; the listing
/// endpoint ships this in place of the full `metadata` blob to
/// keep the wire payload small (the vehicles category otherwise
/// runs ~4 MB/page).
///
/// Internally tagged by `category` so the JSON is self-describing —
/// TypeScript clients can narrow on `summary.category` even when
/// they've forgotten which endpoint they fetched from. Each variant
/// is a per-category typed struct (rather than the original
/// untyped `serde_json::Map`), so the OpenAPI spec → TS client
/// path produces a real discriminated union.
///
/// Per-category field set (curated for browse-page surfaces — full
/// metadata is still available via the detail endpoint):
///
///   - Vehicle:  manufacturer, role, hull_size, focus
///   - Weapon:   manufacturer, size, damage_type, weapon_type
///   - Item:     manufacturer, item_type, grade
///   - Location: system, parent, tag, classification
///
/// Missing / empty fields serialise as `None` and are skipped on
/// the wire (`skip_serializing_if = "Option::is_none"`) so the
/// frontend can use field-presence checks for chip rendering.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "category", rename_all = "snake_case")]
pub enum Summary {
    Vehicle(VehicleSummary),
    Weapon(WeaponSummary),
    Item(ItemSummary),
    Location(LocationSummary),
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub struct VehicleSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull_size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub struct WeaponSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapon_type: Option<String>,
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub struct ItemSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub struct LocationSummary {
    // -- Wave 1 (from api.star-citizen.wiki) -------------------------
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,

    // -- Wave 2 (from starcitizen.tools enrichment, Phase 1) ---------
    //
    // Populated from `metadata.taxonomy_v2.*`. See
    // `crates/starstats-server/src/location_enrichment.rs` and
    // `crates/starstats-core/src/location_taxonomy.rs` for the
    // upstream parsing and the canonical type shapes. Snake-case
    // string values for `tier` and `subtype` mirror the
    // `LocationTier` enum and the open-ended subtype allow-list
    // documented in `docs/PLAN-LOCATION-TAXONOMY-V2.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faction: Option<String>,
}

/// utoipa::ToSchema mirror of
/// [`starstats_core::location_taxonomy::Placement`]. The pure crate
/// can't derive ToSchema (no framework deps); the wire shape lives
/// here so the OpenAPI spec → TS client pipeline produces a real
/// discriminated union for the web layer to narrow on. Same
/// `#[serde(tag = "kind", rename_all = "snake_case")]` so the JSON
/// bytes are byte-identical to the core type.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlacementSchema {
    OnBody { body: String },
    OrbitsBody { body: String },
    LagrangePoint { lagrange: u8, body: String },
    SunwardFrom { body: String },
    AngleFrom { degrees: i16, body: String },
}

/// Build a typed [`Summary`] from a [`ReferenceEntry`]'s
/// `metadata`. Nested wiki shapes (`manufacturer: { name, code }`,
/// `star: { name }`) are flattened to bare strings via a `.name`
/// lookup with a fallback to a flat-string form for the
/// manufacturer case (the wiki occasionally inlines it as a
/// single string — see `parse_handles_string_manufacturer`).
pub fn build_summary(category: ReferenceCategory, metadata: &serde_json::Value) -> Summary {
    let get_str = |key: &str| -> Option<String> {
        metadata
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let get_nested_str = |obj_key: &str, nested_key: &str| -> Option<String> {
        metadata
            .get(obj_key)?
            .get(nested_key)?
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let manuf = || get_nested_str("manufacturer", "name").or_else(|| get_str("manufacturer"));
    match category {
        ReferenceCategory::Vehicle => Summary::Vehicle(VehicleSummary {
            manufacturer: manuf(),
            role: get_str("role").or_else(|| get_str("focus")),
            hull_size: get_str("hull_size").or_else(|| get_str("size")),
            focus: get_str("focus"),
        }),
        ReferenceCategory::Weapon => Summary::Weapon(WeaponSummary {
            manufacturer: manuf(),
            size: get_str("size"),
            damage_type: get_str("damage_type"),
            weapon_type: get_str("type").or_else(|| get_str("kind")),
        }),
        ReferenceCategory::Item => Summary::Item(ItemSummary {
            manufacturer: manuf(),
            item_type: get_str("type").or_else(|| get_str("kind")),
            grade: get_str("grade"),
        }),
        ReferenceCategory::Location => {
            // `taxonomy_v2` is the Phase-1 enrichment blob mirrored
            // INTO the entry's metadata by
            // `ReferenceStore::apply_location_taxonomies`. Reading
            // from metadata (rather than the column directly) keeps
            // `build_summary` taking `&serde_json::Value` like the
            // other categories — no signature churn for ~30 call
            // sites in the rest of the codebase.
            let tx = metadata.get("taxonomy_v2");
            let tx_str = |key: &str| -> Option<String> {
                tx.and_then(|v| v.get(key))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let placement = tx
                .and_then(|v| v.get("placement"))
                .filter(|v| !v.is_null())
                .and_then(|v| serde_json::from_value::<PlacementSchema>(v.clone()).ok());
            Summary::Location(LocationSummary {
                system: get_nested_str("star", "name"),
                parent: get_nested_str("parent", "name"),
                tag: get_nested_str("tag", "name"),
                classification: get_nested_str("type", "classification"),
                tier: tx_str("tier"),
                subtype: tx_str("subtype"),
                placement,
                operator: tx_str("operator"),
                faction: tx_str("faction"),
            })
        }
    }
}

/// Mutates `entries` in place: assigns a slug to every entry,
/// resolving collisions deterministically.
///
/// For locations: prefer the wiki-supplied `metadata.slug` (the
/// wiki already exposes a per-location slug field). For every other
/// category: derive from `display_name` via [`derive_slug`].
///
/// Determinism guarantee — the function internally sorts the input
/// by `class_name` before assigning suffixes. The wiki upstream's
/// page ordering is not contractually stable across runs, so
/// relying on input order would let two re-syncs swap slugs across
/// a collided pair if the upstream re-orders pages. The internal
/// sort means the same `(class_name, display_name, metadata)` set
/// always produces the same slug assignment regardless of how the
/// caller stacked the pages.
///
/// Idempotent: calling twice is a no-op on the second call. Any
/// entry whose `slug` is already `Some(_)` is left untouched, so
/// the function only ever fills in `None` slots. The
/// `apply_slug_collisions_is_idempotent` test covers this.
///
/// Collision policy: within a single category, the lexically-first
/// `class_name` keeps the bare slug; each subsequent class_name
/// that maps to the same base gets `-2`, `-3`, … appended. Cross-
/// category collisions are not material — the URL space is
/// `/kb/{category}/{slug}` so `vehicle/foo` and `weapon/foo` are
/// distinct routes.
/// Commodity `class_name` prefixes. These are the player-facing items a
/// contract can reference — cargo, harvestables, carryables.
///
/// Everything else in the item category is ship components: measured
/// 2026-07-31, `seat` alone is 837 rows (one per ship, e.g.
/// `AEGS_Hammerhead_SCItem_SeatAccess_Captain_Room`). Those share
/// generic display names by nature and are never contract-referenceable,
/// so they are deliberately left alone.
const COMMODITY_PREFIXES: [&str; 2] = ["carryable", "harvestable"];

fn is_commodity(class_name: &str) -> bool {
    let lower = class_name.to_ascii_lowercase();
    COMMODITY_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// Fold the container/form variants of one commodity into a single entry.
///
/// The wiki lists a commodity once per container size AND once per form,
/// each with an IDENTICAL display name. Measured on the live registry,
/// "Sunset Berries" is nine rows:
///
/// ```text
/// Carryable_TBO_FL_1SCU_Commodity_Organic_SunsetBerry   (and 2/4/8/16/24/32 SCU)
/// Harvestable_SunsetBerry
/// harvestable_base_SunsetBerry
/// ```
///
/// The size lives in `class_name` and is dropped from `display_name`, so
/// the nine collapse to one indistinguishable label and the `-2`/`-3`
/// slug suffix becomes the only discriminator. A contract asking for
/// "15 Sunset Berries" means the commodity, not the 16-SCU crate — so
/// there was no correct row to link to and every choice was arbitrary.
///
/// Measured effect on contract-item resolution: 1/10 items resolved
/// uniquely before, 2/10 collapsing sizes alone, **10/10** collapsing
/// variants as this does.
///
/// Keeps the entry whose `class_name` sorts first, for determinism
/// across runs — the same input always yields the same survivor.
pub fn collapse_commodity_variants(entries: &mut Vec<ReferenceEntry>) {
    use std::collections::HashMap;

    // Only the item category has this shape; leave the rest untouched.
    let mut best: HashMap<(ReferenceCategory, String), String> = HashMap::new();
    for e in entries.iter() {
        if !is_commodity(&e.class_name) {
            continue;
        }
        let key = (e.category, e.display_name.trim().to_ascii_lowercase());
        best.entry(key)
            .and_modify(|c| {
                if e.class_name < *c {
                    *c = e.class_name.clone();
                }
            })
            .or_insert_with(|| e.class_name.clone());
    }

    entries.retain(|e| {
        if !is_commodity(&e.class_name) {
            return true;
        }
        let key = (e.category, e.display_name.trim().to_ascii_lowercase());
        best.get(&key).map(|c| *c == e.class_name).unwrap_or(true)
    });
}

pub fn apply_slug_collisions(entries: &mut [ReferenceEntry]) {
    use std::collections::HashMap;

    // Deterministic ordering — sort by class_name so the same set
    // of entries always yields the same suffix assignment even if
    // the upstream reorders pages between runs. Cheap: O(n log n)
    // on a ~20k worst case (items).
    entries.sort_by(|a, b| a.class_name.cmp(&b.class_name));

    // Per-category counter of slugs assigned in this pass. We
    // deliberately DO NOT pre-seed with entries whose slug is
    // already `Some(_)` — the counter only tracks slots filled by
    // this call, so the idempotency contract is "Some(_) entries
    // are skipped" rather than "the counter starts from the
    // historical count" (which the old pre-seed loop got wrong
    // when a batch mixed already-suffixed slugs with new None
    // slugs — see PR review of KB v1).
    let mut counter: HashMap<(ReferenceCategory, String), u32> = HashMap::new();
    for e in entries.iter_mut() {
        if e.slug.is_some() {
            continue;
        }
        let base = if matches!(e.category, ReferenceCategory::Location) {
            // Locations: try wiki's `slug` field first; the
            // wiki-supplied slug is already URL-safe.
            e.metadata
                .get("slug")
                .and_then(|v| v.as_str())
                .map(slugify_ascii)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| derive_slug(&e.display_name, &e.class_name))
        } else {
            derive_slug(&e.display_name, &e.class_name)
        };
        let count = counter.entry((e.category, base.clone())).or_insert(0);
        *count += 1;
        e.slug = Some(if *count == 1 {
            base
        } else {
            format!("{base}-{count}")
        });
    }
}

/// Pull a string field from a JSON object, treating
/// missing/null/non-string/empty/whitespace-only as `None` after
/// trimming. Centralising this keeps the parser shape consistent —
/// the storage layer should never see `Some("")`.
fn string_field(obj: &serde_json::Value, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_page_extracts_all_fields() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    {
                        "id": 1,
                        "name": "Aegis Avenger Stalker",
                        "class_name": "AEGS_Avenger_Stalker",
                        "manufacturer": { "name": "Aegis Dynamics", "code": "AEGS" },
                        "size": "Small",
                        "focus": "Bounty Hunting",
                        "type": "MultiCrew Combat"
                    }
                ],
                "meta": { "current_page": 1, "last_page": 1, "total": 1 }
            }"#,
        )
        .unwrap();

        let parsed = parse_vehicles_page(&json);
        assert_eq!(parsed.len(), 1);
        let v = &parsed[0];
        assert_eq!(v.class_name, "AEGS_Avenger_Stalker");
        assert_eq!(v.display_name, "Aegis Avenger Stalker");
        assert_eq!(v.manufacturer.as_deref(), Some("Aegis Dynamics"));
        // `role` falls back to `focus` when no explicit `role` field
        // exists — mirrors the most common Wiki shape.
        assert_eq!(v.role.as_deref(), Some("Bounty Hunting"));
        assert_eq!(v.focus.as_deref(), Some("Bounty Hunting"));
        assert_eq!(v.hull_size.as_deref(), Some("Small"));
    }

    #[test]
    fn parse_multi_page_walks_each_page_independently() {
        // The parser is per-page — the page-walking loop lives in
        // `WikiReferenceClient::fetch_vehicles`. Synthesise two
        // pages here and concatenate them by hand to prove that two
        // independent calls compose into the expected flat Vec.
        let page1: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    { "name": "Aegis Avenger Stalker", "class_name": "AEGS_Avenger_Stalker" }
                ],
                "meta": { "current_page": 1, "last_page": 2, "total": 2 }
            }"#,
        )
        .unwrap();
        let page2: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    { "name": "Anvil Hornet", "class_name": "ANVL_Hornet_F7C" }
                ],
                "meta": { "current_page": 2, "last_page": 2, "total": 2 }
            }"#,
        )
        .unwrap();

        let mut combined = parse_vehicles_page(&page1);
        combined.extend(parse_vehicles_page(&page2));
        assert_eq!(combined.len(), 2);
        assert_eq!(combined[0].class_name, "AEGS_Avenger_Stalker");
        assert_eq!(combined[1].class_name, "ANVL_Hornet_F7C");
    }

    #[test]
    fn parse_drops_entries_missing_class_name() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    { "name": "No Class Name Here" },
                    { "name": "Empty Class", "class_name": "" },
                    { "name": "Whitespace Class", "class_name": "   " },
                    { "name": "Null Class", "class_name": null },
                    { "name": "Good One", "class_name": "AEGS_Gladius" }
                ]
            }"#,
        )
        .unwrap();

        let parsed = parse_vehicles_page(&json);
        // Only the last entry survives — every other shape lacks a
        // usable join key and is silently dropped.
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].class_name, "AEGS_Gladius");
    }

    #[test]
    fn parse_handles_missing_optional_fields() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    { "class_name": "AEGS_Bare", "name": "Bare Aegis" }
                ]
            }"#,
        )
        .unwrap();

        let parsed = parse_vehicles_page(&json);
        assert_eq!(parsed.len(), 1);
        let v = &parsed[0];
        assert_eq!(v.class_name, "AEGS_Bare");
        assert_eq!(v.display_name, "Bare Aegis");
        assert_eq!(v.manufacturer, None);
        assert_eq!(v.role, None);
        assert_eq!(v.focus, None);
        assert_eq!(v.hull_size, None);
    }

    #[test]
    fn parse_falls_back_display_name_to_class_name() {
        // Half-formed upstream record: no `name` field at all. Better
        // to surface the class name than nothing at all.
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    { "class_name": "AEGS_Mystery" }
                ]
            }"#,
        )
        .unwrap();
        let parsed = parse_vehicles_page(&json);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].display_name, "AEGS_Mystery");
    }

    #[test]
    fn parse_handles_string_manufacturer() {
        // Some upstream records flatten manufacturer into a bare
        // string instead of `{ name, code }`. The parser must accept
        // either shape.
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    {
                        "class_name": "DRAK_Cutlass_Black",
                        "name": "Drake Cutlass Black",
                        "manufacturer": "Drake Interplanetary"
                    }
                ]
            }"#,
        )
        .unwrap();
        let parsed = parse_vehicles_page(&json);
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].manufacturer.as_deref(),
            Some("Drake Interplanetary")
        );
    }

    #[test]
    fn parse_empty_array_returns_empty_vec() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{ "data": [], "meta": { "current_page": 1, "last_page": 1, "total": 0 } }"#,
        )
        .unwrap();
        assert!(parse_vehicles_page(&json).is_empty());
    }

    #[test]
    fn parse_missing_data_array_returns_empty_vec() {
        // Defensive: a malformed upstream response (no `data` field
        // at all) shouldn't panic — it should yield an empty page.
        let json: serde_json::Value = serde_json::from_str(r#"{ "meta": {} }"#).unwrap();
        assert!(parse_vehicles_page(&json).is_empty());
    }

    #[test]
    fn parse_explicit_role_field_wins_over_focus() {
        // When a vehicle has both `role` and `focus`, prefer `role`
        // (the more specific field). `focus` is preserved separately.
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    {
                        "class_name": "AEGS_Vanguard",
                        "name": "Aegis Vanguard",
                        "role": "Heavy Fighter",
                        "focus": "Combat"
                    }
                ]
            }"#,
        )
        .unwrap();
        let parsed = parse_vehicles_page(&json);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].role.as_deref(), Some("Heavy Fighter"));
        assert_eq!(parsed[0].focus.as_deref(), Some("Combat"));
    }

    // -- parse_category_page (generic) --------------------------------

    #[test]
    fn parse_category_page_weapon_lifts_metadata() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    {
                        "id": 42,
                        "class_name": "KLWE_LaserCannon_S2",
                        "name": "Klaus & Werner Sledge II",
                        "manufacturer": { "name": "Klaus & Werner" },
                        "size": "S2",
                        "damage_type": "Energy"
                    }
                ],
                "meta": { "current_page": 1, "last_page": 1 }
            }"#,
        )
        .unwrap();
        let parsed = parse_category_page(&json, ReferenceCategory::Weapon);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].category, ReferenceCategory::Weapon);
        assert_eq!(parsed[0].class_name, "KLWE_LaserCannon_S2");
        assert_eq!(parsed[0].display_name, "Klaus & Werner Sledge II");
        let meta = parsed[0].metadata.as_object().unwrap();
        assert_eq!(meta.get("size").and_then(|v| v.as_str()), Some("S2"));
        assert_eq!(
            meta.get("damage_type").and_then(|v| v.as_str()),
            Some("Energy")
        );
        // Bookkeeping field stripped.
        assert!(meta.get("id").is_none());
    }

    #[test]
    fn parse_category_page_falls_back_to_code() {
        // Locations frequently use `code` rather than `class_name`.
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    { "code": "OOC_Stanton_2_Crusader", "name": "Crusader" }
                ]
            }"#,
        )
        .unwrap();
        let parsed = parse_category_page(&json, ReferenceCategory::Location);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].class_name, "OOC_Stanton_2_Crusader");
        assert_eq!(parsed[0].display_name, "Crusader");
    }

    #[test]
    fn parse_category_page_drops_entries_without_class_identifier() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "data": [
                    { "name": "Mystery item with no ID at all" },
                    { "class_name": "FOO_Bar", "name": "Foo Bar" }
                ]
            }"#,
        )
        .unwrap();
        let parsed = parse_category_page(&json, ReferenceCategory::Item);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].class_name, "FOO_Bar");
    }

    // -- derive_slug + apply_slug_collisions ---------------------------

    #[test]
    fn derive_slug_prefers_display_name() {
        assert_eq!(
            derive_slug("Aegis Avenger Stalker", "AEGS_Avenger_Stalker"),
            "aegis-avenger-stalker"
        );
    }

    #[test]
    fn derive_slug_falls_back_to_class_name_when_display_empty() {
        assert_eq!(
            derive_slug("", "AEGS_Avenger_Stalker"),
            "aegs-avenger-stalker"
        );
    }

    #[test]
    fn derive_slug_strips_punctuation_and_collapses_hyphens() {
        // Ampersand becomes a hyphen; the two adjacent gaps don't
        // produce `--` because the slugifier collapses runs.
        assert_eq!(
            derive_slug("Klaus & Werner Sledge II", "KLWE_LaserCannon_S2"),
            "klaus-werner-sledge-ii"
        );
        assert_eq!(
            derive_slug("Some Weird Name!! With? Special#Chars", "x"),
            "some-weird-name-with-special-chars"
        );
    }

    #[test]
    fn derive_slug_trims_leading_and_trailing_hyphens() {
        assert_eq!(
            derive_slug("   leading/trailing   ", "x"),
            "leading-trailing"
        );
        assert_eq!(derive_slug("---a---b---", "x"), "a-b");
    }

    #[test]
    fn derive_slug_returns_unknown_for_double_empty() {
        // Both fields slugify to empty — defensive last-resort
        // fallback so callers never see `""`.
        assert_eq!(derive_slug("!!!", "@@@"), "unknown");
    }

    #[test]
    fn derive_slug_drops_non_ascii() {
        // No transliteration — non-ASCII letters are dropped. The
        // class_name fallback usually saves us; if everything is
        // non-ASCII, `unknown` is the final safety net.
        assert_eq!(derive_slug("Café Niño", "CAFE_NINO"), "caf-ni-o");
    }

    fn entry(category: ReferenceCategory, class_name: &str, display_name: &str) -> ReferenceEntry {
        ReferenceEntry {
            category,
            class_name: class_name.to_string(),
            display_name: display_name.to_string(),
            slug: None,
            metadata: serde_json::Value::Object(Default::default()),
        }
    }

    #[test]
    fn apply_slug_collisions_assigns_unique_slugs_per_category() {
        let mut batch = vec![
            entry(
                ReferenceCategory::Vehicle,
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
            ),
            entry(
                ReferenceCategory::Vehicle,
                "DRAK_Cutlass_Black",
                "Drake Cutlass Black",
            ),
        ];
        apply_slug_collisions(&mut batch);
        assert_eq!(batch[0].slug.as_deref(), Some("aegis-avenger-stalker"));
        assert_eq!(batch[1].slug.as_deref(), Some("drake-cutlass-black"));
    }

    #[test]
    fn apply_slug_collisions_suffixes_duplicates_deterministically() {
        // Three vehicles with identical display names — order in the
        // input is the order suffixes get assigned.
        let mut batch = vec![
            entry(ReferenceCategory::Vehicle, "A_Foo", "Foo"),
            entry(ReferenceCategory::Vehicle, "B_Foo", "Foo"),
            entry(ReferenceCategory::Vehicle, "C_Foo", "Foo"),
        ];
        apply_slug_collisions(&mut batch);
        assert_eq!(batch[0].slug.as_deref(), Some("foo"));
        assert_eq!(batch[1].slug.as_deref(), Some("foo-2"));
        assert_eq!(batch[2].slug.as_deref(), Some("foo-3"));
    }

    #[test]
    fn collapse_folds_real_sunset_berries_variants_to_one() {
        // The nine REAL rows from the live registry. A contract asking
        // for "15 Sunset Berries" means the commodity, not the 16-SCU
        // crate — before this there was no correct row to link to.
        let mut batch = vec![
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_16SCU_Commodity_Organic_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_1SCU_Commodity_Organic_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_24SCU_Commodity_Organic_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_2SCU_Commodity_Organic_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_32SCU_Commodity_Organic_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_4SCU_Commodity_Organic_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_8SCU_Commodity_Organic_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "Harvestable_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "harvestable_base_SunsetBerry",
                "Sunset Berries",
            ),
        ];
        collapse_commodity_variants(&mut batch);
        assert_eq!(
            batch.len(),
            1,
            "nine variants of one commodity -> one entry"
        );
    }

    #[test]
    fn collapse_leaves_ship_components_alone() {
        // `seat` is 837 rows on the live registry — one per ship. They
        // share a generic display name by nature, are never contract-
        // referenceable, and collapsing them would destroy real
        // distinctions between different ships' components.
        let mut batch = vec![
            entry(
                ReferenceCategory::Item,
                "AEGS_Hammerhead_SCItem_SeatAccess_Captain_Room",
                "Seat",
            ),
            entry(
                ReferenceCategory::Item,
                "3_seat_bench_constellation",
                "Seat",
            ),
            entry(
                ReferenceCategory::Item,
                "AEGS_Hammerhead_SCItem_Engineer_Access",
                "Seat",
            ),
        ];
        collapse_commodity_variants(&mut batch);
        assert_eq!(batch.len(), 3, "ship components must NOT collapse");
    }

    #[test]
    fn collapse_keeps_distinct_commodities_apart() {
        // Only same-NAME variants fold. Two different commodities must
        // survive even though both are carryables.
        let mut batch = vec![
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_1SCU_Commodity_Organic_SunsetBerry",
                "Sunset Berries",
            ),
            entry(
                ReferenceCategory::Item,
                "Carryable_TBO_FL_1SCU_Commodity_Organic_MarokPearl",
                "Marok Gem",
            ),
        ];
        collapse_commodity_variants(&mut batch);
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn collapse_is_deterministic_and_idempotent() {
        // The survivor is the lowest-sorting class_name, so the same
        // input always yields the same row — a different survivor
        // between syncs would repoint every link to this commodity.
        let build = || {
            vec![
                entry(
                    ReferenceCategory::Item,
                    "Carryable_B_SunsetBerry",
                    "Sunset Berries",
                ),
                entry(
                    ReferenceCategory::Item,
                    "Carryable_A_SunsetBerry",
                    "Sunset Berries",
                ),
            ]
        };
        let mut a = build();
        collapse_commodity_variants(&mut a);
        let mut b = build();
        b.reverse();
        collapse_commodity_variants(&mut b);
        assert_eq!(a[0].class_name, "Carryable_A_SunsetBerry");
        assert_eq!(
            a[0].class_name, b[0].class_name,
            "order of input must not change the survivor"
        );

        collapse_commodity_variants(&mut a);
        assert_eq!(a.len(), 1, "running twice must be a no-op");
    }

    #[test]
    fn apply_slug_collisions_is_idempotent() {
        // Running twice must not bump the suffix again — once a slug
        // is assigned, the function leaves it alone.
        let mut batch = vec![
            entry(ReferenceCategory::Vehicle, "A_Foo", "Foo"),
            entry(ReferenceCategory::Vehicle, "B_Foo", "Foo"),
        ];
        apply_slug_collisions(&mut batch);
        let snapshot: Vec<_> = batch.iter().map(|e| e.slug.clone()).collect();
        apply_slug_collisions(&mut batch);
        let after: Vec<_> = batch.iter().map(|e| e.slug.clone()).collect();
        assert_eq!(snapshot, after);
    }

    #[test]
    fn apply_slug_collisions_scopes_by_category() {
        // Same display name in two different categories must NOT
        // collide — the route URL is `/kb/{category}/{slug}` so the
        // same slug across categories is fine. The HashMap key is
        // `(category, slug)` so each category gets its own counter.
        let mut batch = vec![
            entry(ReferenceCategory::Vehicle, "V_Foo", "Foo"),
            entry(ReferenceCategory::Weapon, "W_Foo", "Foo"),
        ];
        apply_slug_collisions(&mut batch);
        // Find each by class_name — the function sorts internally,
        // so we can't index positionally any more.
        let vehicle = batch
            .iter()
            .find(|e| e.class_name == "V_Foo")
            .expect("vehicle entry preserved");
        let weapon = batch
            .iter()
            .find(|e| e.class_name == "W_Foo")
            .expect("weapon entry preserved");
        assert_eq!(vehicle.slug.as_deref(), Some("foo"));
        assert_eq!(weapon.slug.as_deref(), Some("foo"));
    }

    #[test]
    fn apply_slug_collisions_is_stable_across_input_reorderings() {
        // Wiki page order is not contractually stable across runs;
        // the function's internal sort by class_name ensures the
        // same set of entries always yields the same slug
        // assignment regardless of input ordering. This is the
        // critical invariant that keeps user bookmarks pointing at
        // the same entity across re-syncs.
        let make_batch = |order: &[&str]| -> Vec<ReferenceEntry> {
            order
                .iter()
                .map(|c| entry(ReferenceCategory::Vehicle, c, "Foo"))
                .collect()
        };
        let mut a = make_batch(&["A_Foo", "B_Foo", "C_Foo"]);
        let mut b = make_batch(&["C_Foo", "A_Foo", "B_Foo"]);
        let mut c = make_batch(&["B_Foo", "C_Foo", "A_Foo"]);
        apply_slug_collisions(&mut a);
        apply_slug_collisions(&mut b);
        apply_slug_collisions(&mut c);
        for batch in [&a, &b, &c] {
            let lookup: std::collections::HashMap<_, _> = batch
                .iter()
                .map(|e| (e.class_name.as_str(), e.slug.as_deref().unwrap_or("")))
                .collect();
            assert_eq!(lookup["A_Foo"], "foo");
            assert_eq!(lookup["B_Foo"], "foo-2");
            assert_eq!(lookup["C_Foo"], "foo-3");
        }
    }

    #[test]
    fn apply_slug_collisions_prefers_wiki_slug_for_locations() {
        // Locations carry the wiki's own slug in metadata. Use it
        // verbatim (after slugify normalisation) rather than
        // deriving from display_name — this preserves wiki URLs.
        let mut entry_with_wiki_slug = entry(
            ReferenceCategory::Location,
            "OOC_Stanton_2_Crusader",
            "Crusader",
        );
        let mut meta = serde_json::Map::new();
        meta.insert(
            "slug".into(),
            serde_json::Value::String("crusader-prime".into()),
        );
        entry_with_wiki_slug.metadata = serde_json::Value::Object(meta);

        let mut batch = vec![entry_with_wiki_slug];
        apply_slug_collisions(&mut batch);
        assert_eq!(batch[0].slug.as_deref(), Some("crusader-prime"));
    }

    fn as_vehicle(s: Summary) -> VehicleSummary {
        match s {
            Summary::Vehicle(v) => v,
            other => panic!("expected Summary::Vehicle, got {other:?}"),
        }
    }
    fn as_weapon(s: Summary) -> WeaponSummary {
        match s {
            Summary::Weapon(w) => w,
            other => panic!("expected Summary::Weapon, got {other:?}"),
        }
    }
    fn as_location(s: Summary) -> LocationSummary {
        match s {
            Summary::Location(l) => l,
            other => panic!("expected Summary::Location, got {other:?}"),
        }
    }

    #[test]
    fn build_summary_vehicle_projects_curated_fields_and_handles_nested_manufacturer() {
        let meta: serde_json::Value = serde_json::from_str(
            r#"{
                "manufacturer": { "name": "Aegis Dynamics", "code": "AEGS" },
                "role": "Heavy Fighter",
                "hull_size": "Small",
                "focus": "Combat",
                "id": 99,
                "created_at": "ignored"
            }"#,
        )
        .unwrap();
        let s = as_vehicle(build_summary(ReferenceCategory::Vehicle, &meta));
        assert_eq!(s.manufacturer.as_deref(), Some("Aegis Dynamics"));
        assert_eq!(s.role.as_deref(), Some("Heavy Fighter"));
        assert_eq!(s.hull_size.as_deref(), Some("Small"));
        assert_eq!(s.focus.as_deref(), Some("Combat"));
        // Bookkeeping fields (`id`, `created_at`) aren't in the
        // VehicleSummary struct — they can't appear in the projection.
    }

    #[test]
    fn build_summary_vehicle_role_falls_back_to_focus() {
        // Some wiki entries omit `role` and only carry `focus`. The
        // browse UI relies on `role` for the filter chip; the
        // projection coalesces.
        let meta: serde_json::Value = serde_json::from_str(
            r#"{ "manufacturer": "Drake", "focus": "Bounty Hunting", "size": "Small" }"#,
        )
        .unwrap();
        let s = as_vehicle(build_summary(ReferenceCategory::Vehicle, &meta));
        assert_eq!(s.manufacturer.as_deref(), Some("Drake"));
        assert_eq!(s.role.as_deref(), Some("Bounty Hunting"));
        assert_eq!(s.hull_size.as_deref(), Some("Small"));
        assert_eq!(s.focus.as_deref(), Some("Bounty Hunting"));
    }

    #[test]
    fn build_summary_omits_empty_or_missing_fields() {
        // Empty / whitespace-only strings must produce None — the
        // typed struct's `#[serde(skip_serializing_if = "is_none")]`
        // means they vanish from the wire, and the frontend reads
        // optionality directly.
        let meta: serde_json::Value =
            serde_json::from_str(r#"{ "manufacturer": "", "role": "   ", "hull_size": null }"#)
                .unwrap();
        let s = as_vehicle(build_summary(ReferenceCategory::Vehicle, &meta));
        assert!(s.manufacturer.is_none());
        assert!(s.role.is_none());
        assert!(s.hull_size.is_none());
        assert!(s.focus.is_none());
    }

    #[test]
    fn build_summary_weapon_projects_size_and_damage_type() {
        let meta: serde_json::Value = serde_json::from_str(
            r#"{
                "manufacturer": { "name": "Klaus & Werner" },
                "size": "S2",
                "damage_type": "Energy",
                "type": "Laser Cannon"
            }"#,
        )
        .unwrap();
        let s = as_weapon(build_summary(ReferenceCategory::Weapon, &meta));
        assert_eq!(s.manufacturer.as_deref(), Some("Klaus & Werner"));
        assert_eq!(s.size.as_deref(), Some("S2"));
        assert_eq!(s.damage_type.as_deref(), Some("Energy"));
        assert_eq!(s.weapon_type.as_deref(), Some("Laser Cannon"));
    }

    #[test]
    fn build_summary_location_projects_hierarchy_from_nested_wiki_shape() {
        let meta: serde_json::Value = serde_json::from_str(
            r#"{
                "star": { "name": "Stanton" },
                "parent": { "name": "Hurston" },
                "tag": { "name": "Stanton1b" },
                "type": { "classification": "Moon" }
            }"#,
        )
        .unwrap();
        let s = as_location(build_summary(ReferenceCategory::Location, &meta));
        assert_eq!(s.system.as_deref(), Some("Stanton"));
        assert_eq!(s.parent.as_deref(), Some("Hurston"));
        assert_eq!(s.tag.as_deref(), Some("Stanton1b"));
        assert_eq!(s.classification.as_deref(), Some("Moon"));
        // Wave 1 metadata has no `taxonomy_v2` — Wave 2 fields stay
        // None so the wire payload is byte-identical to pre-Phase-1
        // for unenriched rows.
        assert!(s.tier.is_none());
        assert!(s.subtype.is_none());
        assert!(s.placement.is_none());
        assert!(s.operator.is_none());
        assert!(s.faction.is_none());
    }

    #[test]
    fn build_summary_location_picks_up_taxonomy_v2_enrichment() {
        // After `apply_location_taxonomies` mirrors the enrichment
        // blob into `metadata.taxonomy_v2`, `build_summary` must
        // surface every Wave 2 field on the wire so the web /
        // tray layers can render tier chips + spatial-relation
        // tags without round-tripping the full metadata blob.
        let meta: serde_json::Value = serde_json::from_str(
            r#"{
                "star":   { "name": "Stanton" },
                "parent": { "name": "Hurston" },
                "tag":    { "name": "Lorville" },
                "type":   { "classification": "Settlement" },
                "taxonomy_v2": {
                    "tier":     "landing_zone",
                    "subtype":  "city",
                    "placement": { "kind": "on_body", "body": "Hurston" },
                    "operator": "Hurston Dynamics"
                }
            }"#,
        )
        .unwrap();
        let s = as_location(build_summary(ReferenceCategory::Location, &meta));
        // Wave 1 fields still resolve from their original paths.
        assert_eq!(s.system.as_deref(), Some("Stanton"));
        assert_eq!(s.classification.as_deref(), Some("Settlement"));
        // Wave 2 fields come off the enrichment blob.
        assert_eq!(s.tier.as_deref(), Some("landing_zone"));
        assert_eq!(s.subtype.as_deref(), Some("city"));
        assert_eq!(
            s.placement,
            Some(PlacementSchema::OnBody {
                body: "Hurston".into()
            })
        );
        assert_eq!(s.operator.as_deref(), Some("Hurston Dynamics"));
        assert!(s.faction.is_none());
    }

    #[test]
    fn build_summary_location_handles_lagrange_placement() {
        let meta: serde_json::Value = serde_json::from_str(
            r#"{
                "taxonomy_v2": {
                    "tier": "space_station",
                    "subtype": "rest_stop",
                    "placement": {
                        "kind": "lagrange_point",
                        "lagrange": 3,
                        "body": "Terminus"
                    },
                    "faction": "Rough & Ready"
                }
            }"#,
        )
        .unwrap();
        let s = as_location(build_summary(ReferenceCategory::Location, &meta));
        assert_eq!(
            s.placement,
            Some(PlacementSchema::LagrangePoint {
                lagrange: 3,
                body: "Terminus".into()
            })
        );
        assert_eq!(s.faction.as_deref(), Some("Rough & Ready"));
    }

    #[test]
    fn build_summary_handles_missing_metadata_gracefully() {
        let empty = serde_json::Value::Object(Default::default());
        let s = as_vehicle(build_summary(ReferenceCategory::Vehicle, &empty));
        assert_eq!(s, VehicleSummary::default());
    }

    #[test]
    fn summary_serializes_with_category_discriminator() {
        // The internally-tagged enum produces `category` in the
        // JSON so TS clients can narrow on it. None fields are
        // skipped (skip_serializing_if = "Option::is_none").
        let s = Summary::Vehicle(VehicleSummary {
            manufacturer: Some("Aegis".into()),
            role: Some("Fighter".into()),
            hull_size: None,
            focus: None,
        });
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["category"], "vehicle");
        assert_eq!(json["manufacturer"], "Aegis");
        assert_eq!(json["role"], "Fighter");
        assert!(json.get("hull_size").is_none());
        assert!(json.get("focus").is_none());
    }

    #[test]
    fn apply_slug_collisions_preserves_existing_slugs() {
        // An entry that already has a slug keeps it — the function
        // only assigns to entries whose slug is None.
        let mut pre = entry(ReferenceCategory::Vehicle, "X_Foo", "Foo");
        pre.slug = Some("custom-foo".into());
        let mut batch = vec![pre];
        apply_slug_collisions(&mut batch);
        assert_eq!(batch[0].slug.as_deref(), Some("custom-foo"));
    }

    #[test]
    fn parse_category_page_falls_back_to_class_name_for_display() {
        // No `name` field — display defaults to the class name so the
        // dashboard never renders an empty cell.
        let json: serde_json::Value =
            serde_json::from_str(r#"{ "data": [{ "class_name": "ANVL_Hornet_F7C" }] }"#).unwrap();
        let parsed = parse_category_page(&json, ReferenceCategory::Vehicle);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].display_name, "ANVL_Hornet_F7C");
    }
}
