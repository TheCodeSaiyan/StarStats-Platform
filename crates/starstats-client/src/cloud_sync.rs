//! Cloud-sync orchestration on the tray side.
//!
//! Coordinates the side-effects when a user saves config with
//! `sync_with_cloud` changing or already enabled:
//!  - false → true: register with server (POST sync_enabled=true on
//!    own device row), then GET prefs. Adopt on non-empty; seed (PUT)
//!    on empty.
//!  - true → false: POST sync_enabled=false. No GET, no PUT.
//!  - true → true: write-through PUT of the sync-eligible subset.
//!  - false → false: nothing.
//!
//! Caller (the Save command in commands.rs) reads the returned
//! TransitionOutcome to decide whether to re-persist the Config
//! (Adopted) or flip sync_with_cloud back to false (Revoked).

use crate::config::Config;
use crate::config_sync::{apply_remote_prefs, snapshot_for_remote};
use crate::preferences_client::{
    get_preferences, put_preferences, set_device_sync, PreferencesClientError,
};

#[derive(Debug)]
pub enum TransitionOutcome {
    NoOp,
    Seeded,
    /// The server returned non-empty preferences that differ from the
    /// local config. The boxed value is the merged config that the
    /// caller should re-persist and re-emit. Boxed to keep the enum's
    /// memory footprint comparable to the other unit-like variants
    /// (Config is large).
    Adopted(Box<Config>),
    Revoked,
}

pub async fn handle_cloud_sync_transition(
    prev: &Config,
    next: &Config,
    device_id: &str,
) -> Result<TransitionOutcome, anyhow::Error> {
    let client = reqwest::Client::new();
    let api_url = match &next.remote_sync.api_url {
        Some(u) if !u.is_empty() => u.clone(),
        _ => return Ok(TransitionOutcome::NoOp),
    };
    let token = match &next.remote_sync.access_token {
        Some(t) if !t.is_empty() => t.clone(),
        _ => return Ok(TransitionOutcome::NoOp),
    };

    match (prev.sync_with_cloud, next.sync_with_cloud) {
        (false, true) => {
            // Server gate up.
            if let Err(e) = set_device_sync(&client, &api_url, &token, device_id, true).await {
                tracing::warn!(error = %e, "set_device_sync(true) failed on opt-in");
                // Don't return — the local toggle still flips; subsequent
                // GET/PUT will surface the issue.
            }
            // Adopt or seed.
            match get_preferences(&client, &api_url, &token).await {
                Ok(remote) if !is_empty_prefs(&remote) => {
                    let mut adopted = next.clone();
                    let changed = apply_remote_prefs(&mut adopted, &remote);
                    Ok(if changed {
                        TransitionOutcome::Adopted(Box::new(adopted))
                    } else {
                        TransitionOutcome::NoOp
                    })
                }
                Ok(_) => {
                    // Server empty → seed.
                    let snap = snapshot_for_remote(next);
                    if let Err(e) = put_preferences(&client, &api_url, &token, &snap).await {
                        tracing::warn!(error = ?e, "seed PUT failed");
                    }
                    Ok(TransitionOutcome::Seeded)
                }
                Err(PreferencesClientError::SyncDisabled) => Ok(TransitionOutcome::Revoked),
                Err(PreferencesClientError::Other(e)) => {
                    tracing::warn!(error = %e, "opt-in GET failed");
                    Ok(TransitionOutcome::NoOp)
                }
            }
        }
        (true, false) => {
            if let Err(e) = set_device_sync(&client, &api_url, &token, device_id, false).await {
                tracing::warn!(error = %e, "set_device_sync(false) failed on opt-out");
            }
            Ok(TransitionOutcome::NoOp)
        }
        (true, true) => {
            // Write-through PUT.
            let snap = snapshot_for_remote(next);
            match put_preferences(&client, &api_url, &token, &snap).await {
                Ok(_) => Ok(TransitionOutcome::NoOp),
                Err(PreferencesClientError::SyncDisabled) => Ok(TransitionOutcome::Revoked),
                Err(PreferencesClientError::Other(e)) => {
                    tracing::warn!(error = %e, "write-through PUT failed");
                    Ok(TransitionOutcome::NoOp)
                }
            }
        }
        (false, false) => Ok(TransitionOutcome::NoOp),
    }
}

fn is_empty_prefs(p: &starstats_core::wire::UserPreferences) -> bool {
    p.theme.is_none()
        && p.debug_logging.is_none()
        && p.auto_update_check.is_none()
        && p.release_channel.is_none()
        && p.api_url.is_none()
        && p.remote_sync.is_none()
}

/// Extract the `device_id` claim from a device JWT without verifying
/// the signature. The tray trusts its own persisted token; we only
/// need the claim value to construct the URL path for POST
/// /v1/auth/devices/:id/sync.
///
/// The JWT format is `header.payload.signature` where `payload` is
/// base64url (URL_SAFE_NO_PAD) encoded JSON. Returns an error when
/// the token is missing, malformed, or the `device_id` claim is absent.
pub fn extract_device_id_from_token(cfg: &Config) -> anyhow::Result<String> {
    use anyhow::Context;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    let token = cfg
        .remote_sync
        .access_token
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no access_token"))?;
    let payload_seg = token
        .split('.')
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("malformed JWT"))?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload_seg)
        .context("decode JWT payload")?;
    let claims: serde_json::Value =
        serde_json::from_slice(&decoded).context("parse JWT payload")?;
    let device_id = claims
        .get("device_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("device_id claim missing"))?;
    Ok(device_id.to_string())
}

#[cfg(test)]
mod cloud_sync_tests {
    use super::*;
    use crate::config::Config;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_test_config_with_api(server_uri: String, sync_with_cloud: bool) -> Config {
        use crate::config::RemoteSyncConfig;
        Config {
            sync_with_cloud,
            remote_sync: RemoteSyncConfig {
                api_url: Some(server_uri),
                access_token: Some("tok".into()),
                claimed_handle: Some("U".into()),
                ..RemoteSyncConfig::default()
            },
            ..Config::default()
        }
    }

    /// On false→true transition: tray POSTs sync_enabled=true on its
    /// own device row, then GETs preferences. If the response is
    /// non-empty the tray adopts those values; if empty it PUTs the
    /// local snapshot to seed the server.
    #[tokio::test]
    async fn opt_in_transition_seeds_server_when_empty() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/auth/devices/dev-1/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sync_enabled": true
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/me/preferences"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/v1/me/preferences"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let cfg = make_test_config_with_api(server.uri(), true);
        let prev = {
            let mut c = cfg.clone();
            c.sync_with_cloud = false;
            c
        };

        let outcome = handle_cloud_sync_transition(&prev, &cfg, "dev-1")
            .await
            .unwrap();
        // Server returned empty prefs → tray seeded → outcome is Seeded.
        assert!(matches!(outcome, TransitionOutcome::Seeded));
    }

    #[tokio::test]
    async fn opt_in_transition_adopts_when_server_has_different_prefs() {
        use crate::config::Theme;
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/auth/devices/dev-1/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sync_enabled": true
            })))
            .expect(1)
            .mount(&server)
            .await;
        // GET returns a non-empty payload with a theme that differs from
        // the test config's default. Tray should adopt it.
        Mock::given(method("GET"))
            .and(path("/v1/me/preferences"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "theme": "nyx"
            })))
            .expect(1)
            .mount(&server)
            .await;
        // No PUT should fire — the server already has prefs, no seed needed.

        let cfg = make_test_config_with_api(server.uri(), true);
        // Make sure the test cfg's theme is NOT nyx so the adopt is observable.
        // Default theme on Config is Stanton; nyx differs.
        let prev = {
            let mut c = cfg.clone();
            c.sync_with_cloud = false;
            c
        };

        let outcome = handle_cloud_sync_transition(&prev, &cfg, "dev-1")
            .await
            .unwrap();

        match outcome {
            TransitionOutcome::Adopted(boxed) => {
                let adopted = *boxed;
                assert_eq!(adopted.theme, Theme::Nyx);
            }
            other => panic!("expected Adopted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn opt_out_transition_posts_disable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/devices/dev-1/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sync_enabled": false
            })))
            .expect(1)
            .mount(&server)
            .await;
        // Critically: no GET, no PUT — only the disable POST.

        let prev = make_test_config_with_api(server.uri(), true);
        let next = {
            let mut c = prev.clone();
            c.sync_with_cloud = false;
            c
        };

        let outcome = handle_cloud_sync_transition(&prev, &next, "dev-1")
            .await
            .unwrap();
        assert!(matches!(outcome, TransitionOutcome::NoOp));
    }

    #[tokio::test]
    async fn write_through_put_on_unchanged_true() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/v1/me/preferences"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "theme": "pyro"
            })))
            .expect(1)
            .mount(&server)
            .await;
        // No POST sync, no GET — only the write-through PUT.

        let cfg = make_test_config_with_api(server.uri(), true);
        let outcome = handle_cloud_sync_transition(&cfg, &cfg, "dev-1")
            .await
            .unwrap();
        assert!(matches!(outcome, TransitionOutcome::NoOp));
    }

    #[tokio::test]
    async fn no_op_when_both_false() {
        // No mocks set — any HTTP call would fail.
        let cfg = make_test_config_with_api("http://unused.invalid".into(), false);
        let outcome = handle_cloud_sync_transition(&cfg, &cfg, "dev-1")
            .await
            .unwrap();
        assert!(matches!(outcome, TransitionOutcome::NoOp));
    }

    // -- extract_device_id_from_token --

    #[test]
    fn extracts_device_id_from_valid_jwt() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        // Build a minimal JWT: header.payload.signature
        let payload_json = serde_json::json!({
            "device_id": "abc-123",
            "preferred_username": "TestUser"
        });
        let payload_b64 =
            URL_SAFE_NO_PAD.encode(serde_json::to_string(&payload_json).unwrap().as_bytes());
        let token = format!("header.{payload_b64}.sig");

        let mut cfg = Config::default();
        cfg.remote_sync.access_token = Some(token);
        assert_eq!(extract_device_id_from_token(&cfg).unwrap(), "abc-123");
    }

    #[test]
    fn returns_error_when_no_token() {
        let cfg = Config::default(); // access_token is None
        assert!(extract_device_id_from_token(&cfg).is_err());
    }

    #[test]
    fn returns_error_when_device_id_claim_missing() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let payload_json = serde_json::json!({ "preferred_username": "TestUser" });
        let payload_b64 =
            URL_SAFE_NO_PAD.encode(serde_json::to_string(&payload_json).unwrap().as_bytes());
        let token = format!("header.{payload_b64}.sig");

        let mut cfg = Config::default();
        cfg.remote_sync.access_token = Some(token);
        assert!(extract_device_id_from_token(&cfg).is_err());
    }
}
