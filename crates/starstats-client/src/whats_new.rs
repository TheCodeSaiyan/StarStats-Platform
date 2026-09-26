//! Tray-side fetcher for the roadmap "What's new" panel.
//!
//! The WebView's CSP blocks cross-origin `fetch()` from the renderer,
//! so the panel's two HTTP calls must run Rust-side and ride the IPC
//! bridge. This module owns the bearer-auth-aware reqwest client +
//! the wire DTOs, mirroring the server's `whats_new_routes` shapes so
//! the renderer can deserialize the JSON the Tauri command returns
//! without depending on a shared crate.
//!
//! Endpoints touched (server-side spec §9):
//!  - GET  /v1/me/roadmap/whats-new
//!  - POST /v1/me/roadmap/whats-new/seen

use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Distinguished error type so the Tauri-command layer can surface a
/// concrete reason to the React side. Mirrors the
/// `preferences_client.rs` pattern.
#[derive(Debug, thiserror::Error)]
pub enum WhatsNewClientError {
    #[error("tray is not paired (no api_url or token)")]
    NotPaired,
    #[error("server returned {0}")]
    Status(reqwest::StatusCode),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Mirror of `whats_new_routes::WhatsNewItem`. Kept intentionally
/// stable so the renderer (which deserializes the JSON returned by a
/// `#[tauri::command]`) doesn't need a shared schema crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WhatsNewItem {
    pub roadmap_item_id: Uuid,
    pub slug: String,
    pub title: String,
    pub headline_status: String,
    pub latest_changelog_entry_id: Uuid,
    pub latest_published_at: DateTime<Utc>,
    pub unread: bool,
}

/// Mirror of `whats_new_routes::WhatsNewResponse`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WhatsNewResponse {
    pub items: Vec<WhatsNewItem>,
    pub seen_via_auth: bool,
}

/// One line of release notes, as scripts/release-notes.mjs emits it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseLine {
    pub text: String,
    #[serde(default)]
    pub surfaces: Vec<String>,
    #[serde(default)]
    pub prs: Vec<u64>,
    #[serde(default)]
    pub roadmap: Option<String>,
}

/// "New", "Improved" or "Fixed", with its lines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseGroup {
    pub kind: String,
    pub lines: Vec<ReleaseLine>,
}

/// Mirror of `release_routes::MyRelease` (a flattened `Release` plus the
/// player's unread flag).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseItem {
    pub id: Uuid,
    pub track: String,
    pub tag: String,
    pub version: String,
    pub channel: String,
    pub released_on: String,
    pub summary: String,
    pub notes: Vec<ReleaseGroup>,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleasesResponse {
    pub releases: Vec<ReleaseItem>,
    pub unread_count: usize,
}

/// The release channels a player on `channel` sees: their own and every
/// more stable one. An alpha tester also reads beta, rc and live notes;
/// a live player reads live only.
pub fn channels_for(channel: crate::config::ReleaseChannel) -> &'static [&'static str] {
    use crate::config::ReleaseChannel as C;
    match channel {
        C::Alpha => &["alpha", "beta", "rc", "live"],
        C::Beta => &["beta", "rc", "live"],
        C::Rc => &["rc", "live"],
        C::Live => &["live"],
    }
}

/// Mirror of `news_routes::MyNewsItem`: a staff news post with this
/// player's unread flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewsItem {
    pub id: Uuid,
    pub title: String,
    /// Plain text. The pane renders it as text, never as HTML.
    pub body: String,
    pub link_url: Option<String>,
    pub published_at: DateTime<Utc>,
    pub unread: bool,
}

/// Mirror of `news_routes::MyNewsResponse`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewsResponse {
    pub items: Vec<NewsItem>,
    pub unread_count: usize,
}

/// Mirror of the server's `MarkSeenRequest` body.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MarkSeenBody {
    roadmap_item_id: Uuid,
    changelog_entry_id: Uuid,
}

/// Stateless tray-side fetcher. Constructed per command call so config
/// reloads (e.g. after a pair / unpair) immediately reflect in the
/// next request without a long-lived cache.
pub struct WhatsNewClient {
    http: Client,
    api_url: String,
    bearer: Option<String>,
}

impl WhatsNewClient {
    /// Build a client from the current `Config`'s `remote_sync.api_url`
    /// + `access_token`. Returns `NotPaired` if either is missing —
    /// the renderer should treat that as "no panel yet" and fall back
    /// to an empty list (the panel itself has an empty-state branch).
    ///
    /// The 15-second timeout matches `get_reference_category`'s — the
    /// tray's other rust-side relays — so error surfacing is consistent.
    pub fn from_config(cfg: &crate::config::Config) -> Result<Self, WhatsNewClientError> {
        let api_url = cfg
            .remote_sync
            .api_url
            .as_deref()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .ok_or(WhatsNewClientError::NotPaired)?;
        // The anonymous path is supported server-side, so a missing
        // access_token isn't fatal — pass through `bearer = None` and
        // the server falls back to the "recent changes" framing.
        let bearer = cfg
            .remote_sync
            .access_token
            .as_deref()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let http = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("build http client")?;
        Ok(Self {
            http,
            api_url,
            bearer,
        })
    }

    /// GET /v1/me/roadmap/whats-new
    pub async fn fetch_whats_new(&self) -> Result<WhatsNewResponse, WhatsNewClientError> {
        let url = format!("{}/v1/me/roadmap/whats-new", self.api_url);
        let mut req = self.http.get(&url);
        if let Some(t) = &self.bearer {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.context("send GET whats-new")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(WhatsNewClientError::Status(status));
        }
        let body = resp
            .json::<WhatsNewResponse>()
            .await
            .context("decode whats-new response")?;
        Ok(body)
    }

    /// GET /v1/me/news — staff news with this player's unread flags.
    /// Requires a bearer: news read state is per player.
    pub async fn fetch_news(&self) -> Result<NewsResponse, WhatsNewClientError> {
        let bearer = self
            .bearer
            .as_deref()
            .ok_or(WhatsNewClientError::NotPaired)?;
        let url = format!("{}/v1/me/news?limit=10", self.api_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(bearer)
            .send()
            .await
            .context("send GET news")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(WhatsNewClientError::Status(status));
        }
        Ok(resp
            .json::<NewsResponse>()
            .await
            .context("decode news response")?)
    }

    /// GET /v1/me/releases?track=tray&channels=... — tray release notes
    /// with this player's unread flags. Requires a bearer.
    pub async fn fetch_releases(
        &self,
        channels: &[&str],
    ) -> Result<ReleasesResponse, WhatsNewClientError> {
        let bearer = self
            .bearer
            .as_deref()
            .ok_or(WhatsNewClientError::NotPaired)?;
        let url = format!(
            "{}/v1/me/releases?track=tray&limit=5&channels={}",
            self.api_url,
            channels.join(",")
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(bearer)
            .send()
            .await
            .context("send GET releases")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(WhatsNewClientError::Status(status));
        }
        Ok(resp
            .json::<ReleasesResponse>()
            .await
            .context("decode releases response")?)
    }

    /// POST /v1/me/releases/{id}/seen
    pub async fn mark_release_seen(&self, id: Uuid) -> Result<(), WhatsNewClientError> {
        let bearer = self
            .bearer
            .as_deref()
            .ok_or(WhatsNewClientError::NotPaired)?;
        let url = format!("{}/v1/me/releases/{id}/seen", self.api_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(bearer)
            .send()
            .await
            .context("send POST release seen")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(WhatsNewClientError::Status(status));
        }
        Ok(())
    }

    /// POST /v1/me/news/{id}/seen
    pub async fn mark_news_seen(&self, id: Uuid) -> Result<(), WhatsNewClientError> {
        let bearer = self
            .bearer
            .as_deref()
            .ok_or(WhatsNewClientError::NotPaired)?;
        let url = format!("{}/v1/me/news/{id}/seen", self.api_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(bearer)
            .send()
            .await
            .context("send POST news seen")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(WhatsNewClientError::Status(status));
        }
        Ok(())
    }

    /// POST /v1/me/roadmap/whats-new/seen — requires a bearer.
    /// Returns `NotPaired` if the tray has no access token.
    pub async fn mark_seen(
        &self,
        roadmap_item_id: Uuid,
        changelog_entry_id: Uuid,
    ) -> Result<(), WhatsNewClientError> {
        let bearer = self
            .bearer
            .as_deref()
            .ok_or(WhatsNewClientError::NotPaired)?;
        let url = format!("{}/v1/me/roadmap/whats-new/seen", self.api_url);
        let body = MarkSeenBody {
            roadmap_item_id,
            changelog_entry_id,
        };
        let resp = self
            .http
            .post(&url)
            .bearer_auth(bearer)
            .json(&body)
            .send()
            .await
            .context("send POST whats-new/seen")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(WhatsNewClientError::Status(status));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whats_new_response_round_trips_through_json() {
        // Canned JSON matching the server-side `whats_new_routes`
        // serialization. Verifies the field names + types stay aligned
        // — the renderer relies on this exact shape (the Tauri command
        // returns this struct serialized to JSON).
        let raw = r#"{
            "items": [
                {
                    "roadmap_item_id": "01963f37-3aa1-7000-8000-000000000001",
                    "slug": "feature-x",
                    "title": "Feature X",
                    "headline_status": "shipped",
                    "latest_changelog_entry_id": "01963f37-3aa1-7000-8000-000000000002",
                    "latest_published_at": "2026-05-22T12:00:00Z",
                    "unread": true
                }
            ],
            "seen_via_auth": true
        }"#;
        let parsed: WhatsNewResponse =
            serde_json::from_str(raw).expect("deserialize WhatsNewResponse");
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].slug, "feature-x");
        assert!(parsed.items[0].unread);
        assert!(parsed.seen_via_auth);
    }

    #[test]
    fn empty_anonymous_response_round_trips() {
        // Anonymous + nothing-published-yet case.
        let raw = r#"{ "items": [], "seen_via_auth": false }"#;
        let parsed: WhatsNewResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.items.is_empty());
        assert!(!parsed.seen_via_auth);
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;
    use crate::config::ReleaseChannel;

    #[test]
    fn a_player_sees_their_channel_and_every_more_stable_one() {
        assert_eq!(channels_for(ReleaseChannel::Live), &["live"]);
        assert_eq!(channels_for(ReleaseChannel::Rc), &["rc", "live"]);
        assert_eq!(
            channels_for(ReleaseChannel::Alpha),
            &["alpha", "beta", "rc", "live"]
        );
    }

    #[test]
    fn release_payload_matches_the_server_shape() {
        let json = serde_json::json!({
            "releases": [{
                "id": "0199c000-0000-7000-8000-000000000001",
                "track": "tray",
                "tag": "tray-v0.1.31",
                "version": "0.1.31",
                "channel": "live",
                "released_on": "2026-09-26",
                "summary": "1 new, 1 fixed",
                "notes": [{ "kind": "New", "lines": [{ "text": "Friends", "surfaces": ["Tray"], "prs": [134], "roadmap": "friends-notifications" }] }],
                "created_at": "2026-09-26T18:00:00Z",
                "unread": true
            }],
            "unread_count": 1
        });
        let parsed: ReleasesResponse = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.releases[0].notes[0].lines[0].prs, vec![134]);
        assert!(parsed.releases[0].unread);
    }
}
