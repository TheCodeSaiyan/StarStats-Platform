//! GraphQL `ProjectItem` → roadmap-domain mapping.
//!
//! Pure-function module: takes a parsed `ProjectItem` from the
//! GitHub Projects v2 client (Phase 2) and emits a `MappedItem`
//! shaped for the store's `upsert_item` + `upsert_channel_status`
//! calls. The reconciler and the webhook handler (Phase 3 sync) are
//! the consumers.
//!
//! Per spec §3.3, channels and surfaces come from labels on the
//! linked Issue/PR (`channel/*`, `surface/*`), not from custom
//! fields — Projects v2 doesn't support multi-select custom fields.

use std::collections::HashMap;

use super::github_graphql::{ProjectFieldValue, ProjectItem, ProjectItemContent};
use super::models::{ChannelName, EtaBand, RoadmapStatus};

/// Result of mapping one `ProjectItem` into roadmap-domain shape.
/// Owned strings throughout — borrows would impose a lifetime on the
/// caller for no real benefit.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedItem {
    pub github_project_item_id: String,
    pub slug: String,
    pub title: String,
    pub summary: Option<String>,
    pub category: Option<String>,
    pub eta_band: Option<EtaBand>,
    pub surfaces: Vec<String>,
    pub public: bool,
    /// Status custom field on the Project board, if set. The CI
    /// pipeline owns per-channel statuses (§2.5); this field is the
    /// item-level *headline* the board owner sees.
    pub status: Option<RoadmapStatus>,
    /// Channels derived from `channel/*` labels on the linked
    /// Issue/PR. Empty for DraftIssue items (spec §2.6
    /// "not-yet-targeted").
    pub channels: Vec<ChannelName>,
}

// ---------- public entry point ---------------------------------------------

/// Map one `ProjectItem` into a `MappedItem`.
pub fn map_project_item(pi: &ProjectItem) -> MappedItem {
    let title = content_title(&pi.content).unwrap_or_default();
    let body = content_body(&pi.content).unwrap_or_default();
    let labels = content_labels(&pi.content);
    MappedItem {
        github_project_item_id: pi.id.clone(),
        slug: slugify(title),
        title: title.to_string(),
        summary: extract_summary(body),
        category: extract_category(&pi.custom_fields),
        eta_band: extract_eta_band(&pi.custom_fields),
        surfaces: extract_surfaces(labels),
        public: extract_public(&pi.custom_fields),
        status: extract_status(&pi.custom_fields),
        channels: extract_channels(labels),
    }
}

// ---------- content extractors ---------------------------------------------

fn content_title(c: &ProjectItemContent) -> Option<&str> {
    match c {
        ProjectItemContent::Issue { title, .. }
        | ProjectItemContent::PullRequest { title, .. }
        | ProjectItemContent::DraftIssue { title, .. } => Some(title),
        ProjectItemContent::Other { .. } => None,
    }
}

fn content_body(c: &ProjectItemContent) -> Option<&str> {
    match c {
        ProjectItemContent::Issue { body, .. }
        | ProjectItemContent::PullRequest { body, .. }
        | ProjectItemContent::DraftIssue { body, .. } => Some(body),
        ProjectItemContent::Other { .. } => None,
    }
}

fn content_labels(c: &ProjectItemContent) -> &[String] {
    match c {
        ProjectItemContent::Issue { labels, .. }
        | ProjectItemContent::PullRequest { labels, .. } => labels,
        ProjectItemContent::DraftIssue { .. } | ProjectItemContent::Other { .. } => &[],
    }
}

// ---------- field extractors -----------------------------------------------

/// First non-empty paragraph of the body, capped at 200 chars.
/// `None` if the body is empty or only whitespace.
fn extract_summary(body: &str) -> Option<String> {
    let first = body
        .split("\n\n")
        .map(str::trim)
        .find(|para| !para.is_empty())?;
    let cleaned = first.replace('\n', " ");
    let truncated: String = cleaned.chars().take(200).collect();
    if truncated.is_empty() {
        None
    } else {
        Some(truncated)
    }
}

fn extract_category(fields: &HashMap<String, ProjectFieldValue>) -> Option<String> {
    match fields.get("Category")? {
        ProjectFieldValue::SingleSelect { option_name, .. } => Some(option_name.clone()),
        ProjectFieldValue::Text(s) => Some(s.clone()),
        _ => None,
    }
}

fn extract_eta_band(fields: &HashMap<String, ProjectFieldValue>) -> Option<EtaBand> {
    let v = fields.get("ETA Band").or_else(|| fields.get("Eta Band"))?;
    let raw = match v {
        ProjectFieldValue::SingleSelect { option_name, .. } => option_name.to_ascii_lowercase(),
        ProjectFieldValue::Text(s) => s.to_ascii_lowercase(),
        _ => return None,
    };
    EtaBand::parse(&raw)
}

/// Read `Public` field. Single-select with options `Yes` / `No`,
/// defaulting to false when absent or unrecognised (spec §3.3).
fn extract_public(fields: &HashMap<String, ProjectFieldValue>) -> bool {
    match fields.get("Public") {
        Some(ProjectFieldValue::SingleSelect { option_name, .. }) => {
            matches!(
                option_name.to_ascii_lowercase().as_str(),
                "yes" | "true" | "y"
            )
        }
        Some(ProjectFieldValue::Text(s)) => {
            matches!(s.to_ascii_lowercase().as_str(), "yes" | "true" | "y")
        }
        _ => false,
    }
}

fn extract_status(fields: &HashMap<String, ProjectFieldValue>) -> Option<RoadmapStatus> {
    let v = fields.get("Status")?;
    let raw = match v {
        ProjectFieldValue::SingleSelect { option_name, .. } => option_name.to_ascii_lowercase(),
        ProjectFieldValue::Text(s) => s.to_ascii_lowercase(),
        _ => return None,
    };
    RoadmapStatus::parse(&raw)
}

/// Pull `channel/<name>` labels, parse the suffix into `ChannelName`,
/// dedup. Order matches the input label order. Unknown channel names
/// are silently dropped (a typo on the board doesn't crash the
/// reconciler).
pub fn extract_channels(labels: &[String]) -> Vec<ChannelName> {
    let mut seen: Vec<ChannelName> = Vec::new();
    for label in labels {
        if let Some(suffix) = label.strip_prefix("channel/") {
            if let Some(channel) = ChannelName::parse(suffix) {
                if !seen.contains(&channel) {
                    seen.push(channel);
                }
            }
        }
    }
    seen
}

/// Pull `surface/<name>` labels and return the suffixes verbatim. No
/// parsing — surfaces are free-form per spec §1.5 (the route layer
/// validates against the closed set when serving).
pub fn extract_surfaces(labels: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for label in labels {
        if let Some(suffix) = label.strip_prefix("surface/") {
            let suffix = suffix.to_string();
            if !seen.contains(&suffix) {
                seen.push(suffix);
            }
        }
    }
    seen
}

// ---------- slugify --------------------------------------------------------

/// Title → permalink slug. Lowercase, non-alphanumeric → `-`,
/// collapse multiple `-`, strip leading/trailing `-`, truncate to 80
/// chars on a `-` boundary where possible.
///
/// Slug is recorded once on item insert (spec §1.4) and never
/// regenerated, so the only ambiguity from collisions is when two
/// items have the same title at first sight — the store returns
/// `DuplicateSlug` in that case and the caller can append a
/// disambiguator. The reconciler doesn't currently disambiguate;
/// surface this as a known limitation if it becomes a real problem.
pub fn slugify(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut prev_dash = false;
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    // Trim trailing dash if any.
    while out.ends_with('-') {
        out.pop();
    }
    // Hard cap. Cut at the last `-` boundary if doing so doesn't
    // produce an empty string.
    if out.len() > 80 {
        let mut cut = 80;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        if let Some(last_dash) = out.rfind('-') {
            if last_dash > 0 {
                out.truncate(last_dash);
            }
        }
    }
    out
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::github_graphql::{ProjectFieldValue, ProjectItem, ProjectItemContent};
    use super::*;

    fn item_with_labels(labels: Vec<String>) -> ProjectItem {
        ProjectItem {
            id: "PVTI_x".into(),
            content: ProjectItemContent::Issue {
                title: "Voting UI".into(),
                body: "First paragraph.\n\nSecond paragraph.".into(),
                url: "https://example/x".into(),
                labels,
            },
            custom_fields: HashMap::new(),
        }
    }

    #[test]
    fn slugify_handles_punctuation_and_collapses_dashes() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  ---spaces---  "), "spaces");
        assert_eq!(slugify("UPPER case"), "upper-case");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn slugify_truncates_to_80_at_dash_boundary() {
        let long = "the quick brown fox jumps over the lazy dog and then keeps on going forever indefinitely";
        let slug = slugify(long);
        assert!(slug.len() <= 80, "slug={slug} len={}", slug.len());
        assert!(!slug.ends_with('-'));
        // Should cut at a word boundary, so no partial word at the end.
        assert!(!slug.contains("--"));
    }

    #[test]
    fn extract_channels_strips_prefix_and_dedups() {
        let labels = vec![
            "channel/live".into(),
            "channel/beta".into(),
            "area:roadmap".into(),
            "channel/live".into(),
            "channel/unknown".into(), // silently dropped
        ];
        assert_eq!(
            extract_channels(&labels),
            vec![ChannelName::Live, ChannelName::Beta]
        );
    }

    #[test]
    fn extract_channels_handles_tech_preview() {
        let labels = vec!["channel/tech-preview".into()];
        assert_eq!(extract_channels(&labels), vec![ChannelName::TechPreview]);
    }

    #[test]
    fn extract_surfaces_strips_prefix_and_dedups() {
        let labels = vec![
            "surface/tray-whats-new".into(),
            "surface/web-roadmap".into(),
            "channel/live".into(), // ignored
            "surface/tray-whats-new".into(),
        ];
        assert_eq!(
            extract_surfaces(&labels),
            vec!["tray-whats-new".to_string(), "web-roadmap".to_string()]
        );
    }

    #[test]
    fn extract_public_reads_single_select_yes() {
        let mut fields = HashMap::new();
        fields.insert(
            "Public".to_string(),
            ProjectFieldValue::SingleSelect {
                option_name: "Yes".into(),
                option_id: "opt_y".into(),
            },
        );
        assert!(extract_public(&fields));
    }

    #[test]
    fn extract_public_defaults_to_false_when_absent_or_no() {
        let mut fields: HashMap<String, ProjectFieldValue> = HashMap::new();
        assert!(!extract_public(&fields));
        fields.insert(
            "Public".to_string(),
            ProjectFieldValue::SingleSelect {
                option_name: "No".into(),
                option_id: "opt_n".into(),
            },
        );
        assert!(!extract_public(&fields));
    }

    #[test]
    fn extract_status_parses_single_select_option() {
        let mut fields = HashMap::new();
        fields.insert(
            "Status".to_string(),
            ProjectFieldValue::SingleSelect {
                option_name: "in-design".into(),
                option_id: "opt_d".into(),
            },
        );
        assert_eq!(extract_status(&fields), Some(RoadmapStatus::InDesign));
    }

    #[test]
    fn extract_eta_band_is_case_insensitive() {
        let mut fields = HashMap::new();
        fields.insert(
            "ETA Band".to_string(),
            ProjectFieldValue::SingleSelect {
                option_name: "LATER".into(),
                option_id: "opt_l".into(),
            },
        );
        assert_eq!(extract_eta_band(&fields), Some(EtaBand::Later));
    }

    #[test]
    fn extract_summary_returns_first_paragraph_capped() {
        let body = "First paragraph.\n\nSecond paragraph that's longer.";
        assert_eq!(extract_summary(body), Some("First paragraph.".into()));
    }

    #[test]
    fn extract_summary_returns_none_for_empty_body() {
        assert_eq!(extract_summary(""), None);
        assert_eq!(extract_summary("   \n   "), None);
    }

    #[test]
    fn map_project_item_e2e_with_labels_and_fields() {
        let mut fields = HashMap::new();
        fields.insert(
            "Status".into(),
            ProjectFieldValue::SingleSelect {
                option_name: "building".into(),
                option_id: "x".into(),
            },
        );
        fields.insert(
            "Public".into(),
            ProjectFieldValue::SingleSelect {
                option_name: "Yes".into(),
                option_id: "y".into(),
            },
        );
        fields.insert(
            "Category".into(),
            ProjectFieldValue::SingleSelect {
                option_name: "Backend".into(),
                option_id: "z".into(),
            },
        );
        fields.insert(
            "ETA Band".into(),
            ProjectFieldValue::SingleSelect {
                option_name: "next".into(),
                option_id: "n".into(),
            },
        );
        let pi = ProjectItem {
            id: "PVTI_42".into(),
            content: ProjectItemContent::Issue {
                title: "Voting UI v2".into(),
                body: "We want to make voting better.\n\nDetails here.".into(),
                url: "https://example/42".into(),
                labels: vec![
                    "channel/live".into(),
                    "surface/web-roadmap".into(),
                    "channel/beta".into(),
                ],
            },
            custom_fields: fields,
        };
        let mapped = map_project_item(&pi);
        assert_eq!(mapped.github_project_item_id, "PVTI_42");
        assert_eq!(mapped.slug, "voting-ui-v2");
        assert_eq!(mapped.title, "Voting UI v2");
        assert_eq!(
            mapped.summary,
            Some("We want to make voting better.".to_string())
        );
        assert_eq!(mapped.category.as_deref(), Some("Backend"));
        assert_eq!(mapped.eta_band, Some(EtaBand::Next));
        assert_eq!(mapped.surfaces, vec!["web-roadmap".to_string()]);
        assert!(mapped.public);
        assert_eq!(mapped.status, Some(RoadmapStatus::Building));
        assert_eq!(mapped.channels, vec![ChannelName::Live, ChannelName::Beta]);
    }

    #[test]
    fn draft_issue_yields_empty_channels_and_surfaces() {
        let pi = ProjectItem {
            id: "PVTI_d".into(),
            content: ProjectItemContent::DraftIssue {
                title: "A draft".into(),
                body: "Body".into(),
            },
            custom_fields: HashMap::new(),
        };
        let mapped = map_project_item(&pi);
        assert!(mapped.channels.is_empty());
        assert!(mapped.surfaces.is_empty());
        assert_eq!(mapped.title, "A draft");
        assert_eq!(mapped.slug, "a-draft");
    }

    #[test]
    fn unknown_content_yields_blank_title() {
        let pi = ProjectItem {
            id: "PVTI_o".into(),
            content: ProjectItemContent::Other {
                typename: "FutureType".into(),
            },
            custom_fields: HashMap::new(),
        };
        let mapped = map_project_item(&pi);
        assert_eq!(mapped.title, "");
        assert_eq!(mapped.slug, "");
        // Not crashing is the point.
        let _ = item_with_labels(vec![]); // touch helper for coverage
    }
}
