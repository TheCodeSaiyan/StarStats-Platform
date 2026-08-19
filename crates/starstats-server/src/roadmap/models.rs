//! Row types and closed-vocabulary enums for the roadmap pipeline.
//!
//! The spec (`docs/ROADMAP-PIPELINE-SPEC.md`) defines the data model;
//! this module mirrors §1 (RoadmapItem), §2.1 (ChannelStatus), §4.4
//! (event log), §6 (votes + subscribers), §8 (changelog) and §9
//! (user read state). Vocabulary enums are validated application-
//! side per the project's `closed-vocabulary enums` convention --
//! adding a variant does not need a migration.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

// ---------------------------------------------------------------
// Closed-vocabulary enums (`as_str()` + `parse()` round-trip).
// ---------------------------------------------------------------

/// Channel-level lifecycle status (spec §2.2). Used by both the live
/// `roadmap_channel_statuses` row and its archive twin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RoadmapStatus {
    Proposed,
    /// Serialised as `in-design` (kebab-case) to match the spec.
    InDesign,
    Building,
    Beta,
    Shipped,
    /// Sticky. Once set, only a manual board edit can clear it
    /// (§2.5).
    Parked,
}

impl RoadmapStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::InDesign => "in-design",
            Self::Building => "building",
            Self::Beta => "beta",
            Self::Shipped => "shipped",
            Self::Parked => "parked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "proposed" => Self::Proposed,
            "in-design" => Self::InDesign,
            "building" => Self::Building,
            "beta" => Self::Beta,
            "shipped" => Self::Shipped,
            "parked" => Self::Parked,
            _ => return None,
        })
    }

    /// Ordering rank for the headline-status aggregation (§2.3). Lower
    /// = less-shipped. `Parked` is excluded from aggregation and so
    /// has no defined rank here -- callers should drop parked entries
    /// before ranking.
    fn aggregation_rank(self) -> Option<u8> {
        Some(match self {
            Self::Proposed => 0,
            Self::InDesign => 1,
            Self::Building => 2,
            Self::Beta => 3,
            Self::Shipped => 4,
            Self::Parked => return None,
        })
    }
}

/// Pre-release channel names the pipeline tracks (spec §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ChannelName {
    Live,
    Beta,
    /// Release candidate. First-class peer of beta on the
    /// release-track ladder (alpha → beta → rc → live). Wire format
    /// is plain `"rc"` — kebab-case rename has no effect since
    /// the name has no internal capital. Promoted from a status
    /// in v1.x to its own channel in v1.8.9+ so the roadmap pipeline
    /// can target rc-tagged shipments independently of beta.
    Rc,
    Alpha,
    /// Serialised as `tech-preview` (kebab-case).
    TechPreview,
}

impl ChannelName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Beta => "beta",
            Self::Rc => "rc",
            Self::Alpha => "alpha",
            Self::TechPreview => "tech-preview",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "live" => Self::Live,
            "beta" => Self::Beta,
            "rc" => Self::Rc,
            "alpha" => Self::Alpha,
            "tech-preview" => Self::TechPreview,
            _ => return None,
        })
    }
}

/// CI-derived build health for a given channel (spec §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BuildHealth {
    Passing,
    Failing,
    InProgress,
    Unknown,
}

impl BuildHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passing => "passing",
            Self::Failing => "failing",
            Self::InProgress => "in-progress",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "passing" => Self::Passing,
            "failing" => Self::Failing,
            "in-progress" => Self::InProgress,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

/// ETA hint manually set on the Project board (spec §1.2). The
/// pipeline does NOT derive this from CI signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EtaBand {
    Now,
    Next,
    Later,
    Someday,
    Tbd,
}

impl EtaBand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Now => "now",
            Self::Next => "next",
            Self::Later => "later",
            Self::Someday => "someday",
            Self::Tbd => "tbd",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "now" => Self::Now,
            "next" => Self::Next,
            "later" => Self::Later,
            "someday" => Self::Someday,
            "tbd" => Self::Tbd,
            _ => return None,
        })
    }
}

/// Closed vocabulary for the `kind` field of each `links` entry
/// (spec §1.7). Anything else is rejected at the API layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Pr,
    Issue,
    Doc,
    External,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Issue => "issue",
            Self::Doc => "doc",
            Self::External => "external",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pr" => Self::Pr,
            "issue" => Self::Issue,
            "doc" => Self::Doc,
            "external" => Self::External,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------
// Row structs.
// ---------------------------------------------------------------

/// One row of `roadmap_items` (spec §1.1).
///
/// `category` / `eta_band` are TEXT at the DB layer; this struct
/// keeps them as `Option<String>` so a parse error on a stored value
/// never crashes the read path -- the route layer is responsible for
/// surfacing a domain-error chip on out-of-vocabulary values.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoadmapItem {
    pub id: Uuid,
    pub slug: String,
    pub github_project_item_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub category: Option<String>,
    pub eta_band: Option<String>,
    pub votes: i32,
    pub surfaces: Vec<String>,
    pub parent_id: Option<Uuid>,
    pub links: Value,
    pub public: bool,
    pub content_last_updated: DateTime<Utc>,
    pub pipeline_last_updated: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// One row of `roadmap_channel_statuses` (spec §2.1).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelStatus {
    pub roadmap_item_id: Uuid,
    pub channel: ChannelName,
    pub status: RoadmapStatus,
    pub build_health: BuildHealth,
    pub build_id: Option<String>,
    pub commit_sha: Option<String>,
    pub deployed_at: Option<DateTime<Utc>>,
    pub ci_run_url: Option<String>,
    pub previous_shipped_sha: Option<String>,
    pub last_event_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// One row of `roadmap_channel_statuses_archive` (spec §2.6). Shape
/// matches `ChannelStatus` plus an `archived_at` stamp.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelStatusArchive {
    pub roadmap_item_id: Uuid,
    pub channel: ChannelName,
    pub status: RoadmapStatus,
    pub build_health: BuildHealth,
    pub build_id: Option<String>,
    pub commit_sha: Option<String>,
    pub deployed_at: Option<DateTime<Utc>>,
    pub ci_run_url: Option<String>,
    pub previous_shipped_sha: Option<String>,
    pub last_event_id: Option<String>,
    pub archived_at: DateTime<Utc>,
}

/// One row of `roadmap_event_log` (spec §4.4 -- idempotency).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoadmapEventLogEntry {
    pub event_id: String,
    pub received_at: DateTime<Utc>,
}

/// One row of `roadmap_votes` (spec §6.1).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoadmapVote {
    pub user_id: Uuid,
    pub roadmap_item_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// One row of `roadmap_subscribers` (spec §6.2). Membership is
/// sensitive -- never serialised to public surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoadmapSubscriber {
    pub user_id: Uuid,
    pub roadmap_item_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// One row of `roadmap_changelog` (spec §8). Draft state is
/// `published_at IS NULL`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoadmapChangelogEntry {
    pub id: Uuid,
    pub roadmap_item_id: Uuid,
    pub channel: ChannelName,
    pub title: String,
    pub body: String,
    pub previous_shipped_sha: Option<String>,
    pub shipped_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub published_by: Option<String>,
}

/// One row of `roadmap_user_read_state` (spec §9).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoadmapUserReadState {
    pub user_id: Uuid,
    pub roadmap_item_id: Uuid,
    pub last_seen_changelog_entry_id: Option<Uuid>,
    pub last_seen_at: DateTime<Utc>,
}

// ---------------------------------------------------------------
// Headline-status aggregation (spec §2.3).
// ---------------------------------------------------------------

/// Compute the headline status shown on a roadmap card from its
/// per-channel statuses (spec §2.3).
///
/// Rules:
///   1. Drop channels in `Parked` from the aggregation.
///   2. If nothing remains, the headline is `Parked`.
///   3. Otherwise, take the minimum of remaining channels' statuses
///      under the order
///      `proposed < in-design < building < beta < shipped`.
///
/// Empty input (no channels at all) is treated as `Proposed`. The
/// spec doesn't pin this case explicitly -- the most defensible
/// default is the lowest non-parked rank, since "no channels yet"
/// reads as "freshly proposed" rather than "parked." A board edit
/// that adds a channel later will reset the headline by the normal
/// rule.
pub fn compute_headline_status(channels: &[ChannelStatus]) -> RoadmapStatus {
    if channels.is_empty() {
        return RoadmapStatus::Proposed;
    }
    let remaining: Vec<RoadmapStatus> = channels
        .iter()
        .map(|c| c.status)
        .filter(|s| !matches!(s, RoadmapStatus::Parked))
        .collect();
    if remaining.is_empty() {
        return RoadmapStatus::Parked;
    }
    // The minimum rank wins -- aggregation_rank is `Some` for every
    // non-parked variant, so `expect` here is structurally safe.
    remaining
        .into_iter()
        .min_by_key(|s| s.aggregation_rank().expect("non-parked status has a rank"))
        .expect("at least one element remains after filtering")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(status: RoadmapStatus) -> ChannelStatus {
        ChannelStatus {
            roadmap_item_id: Uuid::nil(),
            channel: ChannelName::Live,
            status,
            build_health: BuildHealth::Unknown,
            build_id: None,
            commit_sha: None,
            deployed_at: None,
            ci_run_url: None,
            previous_shipped_sha: None,
            last_event_id: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn roadmap_status_round_trips() {
        for s in [
            RoadmapStatus::Proposed,
            RoadmapStatus::InDesign,
            RoadmapStatus::Building,
            RoadmapStatus::Beta,
            RoadmapStatus::Shipped,
            RoadmapStatus::Parked,
        ] {
            assert_eq!(RoadmapStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(RoadmapStatus::parse("bogus"), None);
    }

    #[test]
    fn roadmap_status_serde_uses_kebab_case() {
        // The dashed variants are the load-bearing ones; check both.
        let v = serde_json::to_string(&RoadmapStatus::InDesign).unwrap();
        assert_eq!(v, "\"in-design\"");
        let r: RoadmapStatus = serde_json::from_str("\"in-design\"").unwrap();
        assert_eq!(r, RoadmapStatus::InDesign);
    }

    #[test]
    fn channel_name_round_trips_and_serdes_tech_preview() {
        for c in [
            ChannelName::Live,
            ChannelName::Beta,
            ChannelName::Alpha,
            ChannelName::Rc,
            ChannelName::TechPreview,
        ] {
            assert_eq!(ChannelName::parse(c.as_str()), Some(c));
        }
        let v = serde_json::to_string(&ChannelName::TechPreview).unwrap();
        assert_eq!(v, "\"tech-preview\"");
    }

    #[test]
    fn channel_name_rc_serdes_as_lowercase_rc() {
        // Per spec §2.1, rc is its own first-class channel (peer of
        // alpha / beta / live / tech-preview). Wire format is plain
        // lowercase "rc", matching the CI tag-suffix convention.
        assert_eq!(ChannelName::Rc.as_str(), "rc");
        assert_eq!(ChannelName::parse("rc"), Some(ChannelName::Rc));
        let v = serde_json::to_string(&ChannelName::Rc).unwrap();
        assert_eq!(v, "\"rc\"");
        let back: ChannelName = serde_json::from_str("\"rc\"").unwrap();
        assert_eq!(back, ChannelName::Rc);
    }

    #[test]
    fn build_health_round_trips() {
        for h in [
            BuildHealth::Passing,
            BuildHealth::Failing,
            BuildHealth::InProgress,
            BuildHealth::Unknown,
        ] {
            assert_eq!(BuildHealth::parse(h.as_str()), Some(h));
        }
    }

    #[test]
    fn eta_band_round_trips() {
        for b in [
            EtaBand::Now,
            EtaBand::Next,
            EtaBand::Later,
            EtaBand::Someday,
            EtaBand::Tbd,
        ] {
            assert_eq!(EtaBand::parse(b.as_str()), Some(b));
        }
    }

    #[test]
    fn link_kind_round_trips() {
        for k in [
            LinkKind::Pr,
            LinkKind::Issue,
            LinkKind::Doc,
            LinkKind::External,
        ] {
            assert_eq!(LinkKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn headline_status_aggregation_matrix() {
        // All shipped -> shipped.
        let all_shipped = [
            cs(RoadmapStatus::Shipped),
            cs(RoadmapStatus::Shipped),
            cs(RoadmapStatus::Shipped),
        ];
        assert_eq!(
            compute_headline_status(&all_shipped),
            RoadmapStatus::Shipped
        );

        // Mix of shipped + building -> building (min wins).
        let mixed = [cs(RoadmapStatus::Shipped), cs(RoadmapStatus::Building)];
        assert_eq!(compute_headline_status(&mixed), RoadmapStatus::Building);

        // All parked -> parked.
        let all_parked = [cs(RoadmapStatus::Parked), cs(RoadmapStatus::Parked)];
        assert_eq!(compute_headline_status(&all_parked), RoadmapStatus::Parked);

        // Parked + shipped -> shipped (parked excluded from
        // aggregation per §2.3 step 1).
        let parked_and_shipped = [cs(RoadmapStatus::Parked), cs(RoadmapStatus::Shipped)];
        assert_eq!(
            compute_headline_status(&parked_and_shipped),
            RoadmapStatus::Shipped
        );

        // Empty channel list -> proposed (defensive default,
        // documented on compute_headline_status).
        assert_eq!(compute_headline_status(&[]), RoadmapStatus::Proposed);

        // Bonus: proposed + in-design + building -> proposed.
        let asc = [
            cs(RoadmapStatus::Proposed),
            cs(RoadmapStatus::InDesign),
            cs(RoadmapStatus::Building),
        ];
        assert_eq!(compute_headline_status(&asc), RoadmapStatus::Proposed);

        // Bonus: parked + in-design -> in-design (parked dropped).
        let parked_indesign = [cs(RoadmapStatus::Parked), cs(RoadmapStatus::InDesign)];
        assert_eq!(
            compute_headline_status(&parked_indesign),
            RoadmapStatus::InDesign
        );
    }
}
