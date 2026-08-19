//! Thin client wrappers for the tray's cloud-sync calls. Uses the
//! same reqwest::Client and bearer-token plumbing as `sync.rs`.
//!
//! Endpoints touched:
//!  - GET /v1/me/preferences
//!  - PUT /v1/me/preferences
//!  - POST /v1/auth/devices/:id/sync

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use starstats_core::wire::UserPreferences;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSyncRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSyncResponse {
    pub sync_enabled: bool,
}

/// Distinguished error so the caller can tell "the server forbade
/// this call" (revocation path) from any other failure.
#[derive(Debug, thiserror::Error)]
pub enum PreferencesClientError {
    #[error("device sync is disabled by the server")]
    SyncDisabled,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub async fn get_preferences(
    client: &Client,
    api_url: &str,
    bearer: &str,
) -> Result<UserPreferences, PreferencesClientError> {
    let url = format!("{}/v1/me/preferences", api_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .bearer_auth(bearer)
        .send()
        .await
        .context("send GET /v1/me/preferences")?;
    let status = resp.status();
    if status == StatusCode::FORBIDDEN {
        return Err(PreferencesClientError::SyncDisabled);
    }
    if !status.is_success() {
        return Err(PreferencesClientError::Other(anyhow::anyhow!(
            "GET preferences returned {status}"
        )));
    }
    resp.json::<UserPreferences>()
        .await
        .context("decode preferences body")
        .map_err(PreferencesClientError::Other)
}

pub async fn put_preferences(
    client: &Client,
    api_url: &str,
    bearer: &str,
    prefs: &UserPreferences,
) -> Result<UserPreferences, PreferencesClientError> {
    let url = format!("{}/v1/me/preferences", api_url.trim_end_matches('/'));
    let resp = client
        .put(&url)
        .bearer_auth(bearer)
        .json(prefs)
        .send()
        .await
        .context("send PUT /v1/me/preferences")?;
    let status = resp.status();
    if status == StatusCode::FORBIDDEN {
        return Err(PreferencesClientError::SyncDisabled);
    }
    if !status.is_success() {
        return Err(PreferencesClientError::Other(anyhow::anyhow!(
            "PUT preferences returned {status}"
        )));
    }
    resp.json::<UserPreferences>()
        .await
        .context("decode PUT preferences body")
        .map_err(PreferencesClientError::Other)
}

pub async fn set_device_sync(
    client: &Client,
    api_url: &str,
    bearer: &str,
    device_id: &str,
    enabled: bool,
) -> Result<SetSyncResponse> {
    let url = format!(
        "{}/v1/auth/devices/{}/sync",
        api_url.trim_end_matches('/'),
        device_id,
    );
    let resp = client
        .post(&url)
        .bearer_auth(bearer)
        .json(&SetSyncRequest { enabled })
        .send()
        .await
        .context("send POST set device sync")?;
    if !resp.status().is_success() {
        anyhow::bail!("set_device_sync returned {}", resp.status());
    }
    let body = resp
        .json::<SetSyncResponse>()
        .await
        .context("decode set_device_sync body")?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn get_preferences_returns_payload() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me/preferences"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"theme":"pyro"})),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let prefs = get_preferences(&client, &server.uri(), "tok")
            .await
            .unwrap();
        assert_eq!(prefs.theme.as_deref(), Some("pyro"));
    }

    #[tokio::test]
    async fn get_preferences_maps_403_to_sync_disabled() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me/preferences"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "error": "device_sync_disabled"
            })))
            .mount(&server)
            .await;

        let client = Client::new();
        let err = get_preferences(&client, &server.uri(), "tok")
            .await
            .unwrap_err();
        assert!(matches!(err, PreferencesClientError::SyncDisabled));
    }

    #[tokio::test]
    async fn set_device_sync_posts_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/devices/dev-xyz/sync"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"sync_enabled":true})),
            )
            .mount(&server)
            .await;

        let client = Client::new();
        let out = set_device_sync(&client, &server.uri(), "tok", "dev-xyz", true)
            .await
            .unwrap();
        assert!(out.sync_enabled);
    }
}
