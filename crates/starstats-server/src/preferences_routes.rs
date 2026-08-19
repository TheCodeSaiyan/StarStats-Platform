//! Per-user UI preferences endpoints.
//!
//! `GET /v1/me/preferences` returns the caller's stored preferences
//! (or an empty `UserPreferences` payload when nothing has been
//! persisted yet — clients should treat absent fields as "use the
//! default" rather than 404'ing on first load).
//!
//! `PUT /v1/me/preferences` replaces the stored payload in full. The
//! body is a `UserPreferences` and is validated server-side: the
//! current allowlist for `theme` is `{stanton, pyro, terra, nyx}`.
//! Unknown themes return 400 `invalid_theme` rather than silently
//! storing — keeping the JSONB column from accumulating typos.
//!
//! Both endpoints accept user tokens and device tokens. User tokens
//! are unconditionally allowed (`sub`-scoped row lookup). Device
//! tokens must additionally have `devices.sync_enabled = true` for
//! their own row — flipped on by the tray's Cloud sync toggle or
//! from the Connected Uplinks page on the web. A device with
//! sync_enabled = false receives `403 device_sync_disabled` and is
//! expected to surface a notice prompting the user to re-enable.
//! See `the release design notes` §4.

use crate::api_error::ApiErrorBody;
use crate::auth::{AuthenticatedUser, TokenType};
use crate::devices::DeviceStore;
use crate::preferences_store::PreferencesStore;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use url::Url;
use utoipa::ToSchema;
use uuid::Uuid;

/// Body cap on the PUT body. Raised to 4 KB to give headroom for
/// kb_layout (a later task) and any other forward-extensible fields;
/// the current schema comfortably fits in a few hundred bytes so the
/// ceiling stays generous while bounding a runaway client.
const MAX_BODY_BYTES: usize = 4096; // was 1024 — kb_layout (a later task) needs headroom

/// Theme allowlist — must match the four front-end themes the
/// frontend ships. The Postgres column is unconstrained JSONB
/// (deliberately, see migration 0015) so the gate lives here. Order
/// is alphabetical for stable error messages; the set is small enough
/// that linear scan is faster than a HashSet.
const ALLOWED_THEMES: &[&str] = &["nyx", "pyro", "stanton", "terra"];

/// Release-channel allowlist — must match the tray's `ReleaseChannel`
/// enum variants. Order alphabetical for stable error messages.
const ALLOWED_RELEASE_CHANNELS: &[&str] = &["alpha", "beta", "live", "rc"];

/// KB detail view mode allowlist. Order alphabetical for stable error messages.
const ALLOWED_KB_VIEWS: &[&str] = &["compact", "visual"];

/// KB units preference allowlist. Order alphabetical for stable error messages.
const ALLOWED_KB_UNITS: &[&str] = &["imperial", "metric"];

/// Theme-switch wave animation speed allowlist. Must match the web's
/// `WAVE_SPEED_MS` map and the tray-parity `theme_wave_speed` enum.
/// Order alphabetical for stable error messages.
const ALLOWED_WAVE_SPEEDS: &[&str] = &["fast", "normal", "off", "slow"];

const MAX_API_URL_LEN: usize = 256;

fn is_valid_priority_interval(v: u32) -> bool {
    (1..=60).contains(&v)
}

fn is_valid_bulk_interval(v: u32) -> bool {
    (5..=3600).contains(&v)
}

fn is_valid_batch_size(v: u32) -> bool {
    (1..=5000).contains(&v)
}

fn is_valid_api_url(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_API_URL_LEN {
        return false;
    }
    Url::parse(s)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false)
}

// Schema-only mirror of `starstats_core::wire::UserPreferences`. The
// real type lives in `starstats-core`, which has no `utoipa` dep —
// same pattern as `hangar_routes::HangarPushRequestSchema`. Keep this
// in sync with the core type; drift here silently breaks the OpenAPI
// clients without a compile error.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UserPreferencesSchema {
    /// Active theme. One of `stanton`, `pyro`, `terra`, `nyx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Daily-rolling client.log toggle for the tray. Absent → leave stored value alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_logging: Option<bool>,
    /// Tray's "check for updates on launch" preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_update_check: Option<bool>,
    /// Release channel the tray tracks. Validated against the server's
    /// ReleaseChannel enum at write time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_channel: Option<String>,
    /// API URL the tray targets. Validated as a parseable URL ≤ 256 chars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Cadence + transport prefs for the tray's remote sync lane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_sync: Option<RemoteSyncPrefsSchema>,
    /// KB detail view mode. One of `visual`, `compact`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_view: Option<String>,
    /// IANA timezone name, e.g. `Europe/London`. Validated against the tz
    /// database at write time. Absent → the server makes no clock-time
    /// claims about this player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// KB units. One of `metric`, `imperial`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_units: Option<String>,
    /// Theme-switch wave animation speed. One of `off`, `slow`, `normal`,
    /// `fast`. Absent → fall back to the sitewide appearance default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_wave_speed: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct RemoteSyncPrefsSchema {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_interval_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u32>,
}

/// Build the `/v1/me/preferences` sub-router. Per-IP rate limited
/// (1/s sustained, burst 5) — matches `hangar_routes`. Realistic
/// usage is one PUT per theme switch (rare) and one GET per page
/// load, so the limit is generous for legitimate traffic while
/// bounding a runaway client.
pub fn routes<S: PreferencesStore, D: DeviceStore>(prefs: Arc<S>, devices: Arc<D>) -> Router {
    let governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(5)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("preferences governor config builder produced no config"),
    );
    Router::new()
        .route(
            "/v1/me/preferences",
            routing::get(get::<S, D>).put(put::<S, D>),
        )
        .with_state((prefs, devices))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(GovernorLayer { config: governor })
}

fn error(status: StatusCode, code: &'static str, detail: Option<String>) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail,
        }),
    )
        .into_response()
}

/// For device tokens, look up the device row and require
/// `sync_enabled = true`. User tokens fall through without a check.
/// Returns `Some(response)` when the request should be short-circuited.
async fn enforce_device_sync_gate<D: DeviceStore>(
    auth: &AuthenticatedUser,
    devices: &Arc<D>,
) -> Option<Response> {
    if !matches!(auth.token_type, TokenType::Device) {
        return None;
    }
    let device_id = match auth.device_id {
        Some(id) => id,
        None => {
            // Defensive: the auth extractor should have refused a
            // device token without the claim, but never return 500
            // silently on that path.
            return Some(error(
                StatusCode::FORBIDDEN,
                "device_sync_disabled",
                Some("device token without device_id claim".into()),
            ));
        }
    };
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => {
            return Some(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "bad_subject",
                None,
            ));
        }
    };
    match devices.sync_enabled_for(user_id, device_id).await {
        Ok(Some(true)) => None,
        Ok(Some(false)) | Ok(None) => Some(error(
            StatusCode::FORBIDDEN,
            "device_sync_disabled",
            Some(
                "this uplink's sync is disabled — re-enable from the \
                 Connected Uplinks page or from the tray's Cloud sync toggle"
                    .into(),
            ),
        )),
        Err(e) => {
            tracing::error!(error = ?e, device_id = %device_id, "sync_enabled lookup failed");
            Some(error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None))
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/preferences",
    tag = "preferences",
    operation_id = "preferences_get",
    responses(
        (status = 200, description = "Stored preferences (empty object when none set)", body = UserPreferencesSchema),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is a device token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get<S: PreferencesStore, D: DeviceStore>(
    State((store, devices)): State<(Arc<S>, Arc<D>)>,
    auth: AuthenticatedUser,
) -> Response {
    if let Some(resp) = enforce_device_sync_gate(&auth, &devices).await {
        return resp;
    }

    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    match store.get(user_id).await {
        Ok(prefs) => (StatusCode::OK, Json(prefs)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "preferences get failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1/me/preferences",
    tag = "preferences",
    operation_id = "preferences_put",
    request_body = UserPreferencesSchema,
    responses(
        (status = 200, description = "Preferences stored", body = UserPreferencesSchema),
        (status = 400, description = "Invalid theme or malformed body", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is a device token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn put<S: PreferencesStore, D: DeviceStore>(
    State((store, devices)): State<(Arc<S>, Arc<D>)>,
    auth: AuthenticatedUser,
    Json(body): Json<starstats_core::wire::UserPreferences>,
) -> Response {
    if let Some(resp) = enforce_device_sync_gate(&auth, &devices).await {
        return resp;
    }

    if let Some(theme) = body.theme.as_deref() {
        if !ALLOWED_THEMES.contains(&theme) {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_theme",
                Some(format!(
                    "theme must be one of {ALLOWED_THEMES:?}; got {theme:?}"
                )),
            );
        }
    }

    if let Some(ch) = body.release_channel.as_deref() {
        if !ALLOWED_RELEASE_CHANNELS.contains(&ch) {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_channel",
                Some(format!(
                    "release_channel must be one of {ALLOWED_RELEASE_CHANNELS:?}; got {ch:?}"
                )),
            );
        }
    }

    if let Some(url) = body.api_url.as_deref() {
        if !is_valid_api_url(url) {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_api_url",
                Some(format!(
                    "api_url must be a valid http(s) URL ≤ {MAX_API_URL_LEN} chars"
                )),
            );
        }
    }

    if let Some(view) = body.kb_view.as_deref() {
        if !ALLOWED_KB_VIEWS.contains(&view) {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_kb_view",
                Some(format!(
                    "kb_view must be one of {ALLOWED_KB_VIEWS:?}; got {view:?}"
                )),
            );
        }
    }
    if let Some(units) = body.kb_units.as_deref() {
        if !ALLOWED_KB_UNITS.contains(&units) {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_kb_units",
                Some(format!(
                    "kb_units must be one of {ALLOWED_KB_UNITS:?}; got {units:?}"
                )),
            );
        }
    }
    if let Some(tz) = body.timezone.as_deref() {
        // Validated against the real tz database rather than a regex: the
        // whole point of storing a name instead of an offset is that the
        // database resolves DST, so a name it does not know is useless to
        // us. Rejecting at write time keeps the read path total.
        if tz.parse::<chrono_tz::Tz>().is_err() {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_timezone",
                Some(format!(
                    "timezone must be an IANA name such as Europe/London; got {tz:?}"
                )),
            );
        }
    }
    if let Some(speed) = body.theme_wave_speed.as_deref() {
        if !ALLOWED_WAVE_SPEEDS.contains(&speed) {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_wave_speed",
                Some(format!(
                    "theme_wave_speed must be one of {ALLOWED_WAVE_SPEEDS:?}; got {speed:?}"
                )),
            );
        }
    }

    if let Some(rs) = body.remote_sync.as_ref() {
        if let Some(v) = rs.priority_interval_secs {
            if !is_valid_priority_interval(v) {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_priority_interval",
                    Some("priority_interval_secs must be 1..=60".into()),
                );
            }
        }
        if let Some(v) = rs.interval_secs {
            if !is_valid_bulk_interval(v) {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_interval",
                    Some("interval_secs must be 5..=3600".into()),
                );
            }
        }
        if let Some(v) = rs.batch_size {
            if !is_valid_batch_size(v) {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_batch_size",
                    Some("batch_size must be 1..=5000".into()),
                );
            }
        }
    }

    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    match store.put(user_id, &body).await {
        Ok(()) => (StatusCode::OK, Json(body)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "preferences put failed in /v1/me/preferences");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthenticatedUser, TokenType};
    use crate::devices::DeviceStore;
    use crate::preferences_store::test_support::MemoryPreferencesStore;
    use starstats_core::wire::UserPreferences;

    fn make_user_token_auth(user_id: Uuid) -> AuthenticatedUser {
        AuthenticatedUser {
            sub: user_id.to_string(),
            preferred_username: "TestUser".to_string(),
            token_type: TokenType::User,
            device_id: None,
        }
    }

    fn make_device_token_auth(user_id: Uuid, device_id: Uuid) -> AuthenticatedUser {
        AuthenticatedUser {
            sub: user_id.to_string(),
            preferred_username: "TestUser".to_string(),
            token_type: TokenType::Device,
            device_id: Some(device_id),
        }
    }

    async fn get_handler_inner<S: PreferencesStore, D: DeviceStore>(
        prefs: Arc<S>,
        devices: Arc<D>,
        auth: AuthenticatedUser,
    ) -> Response {
        get(State((prefs, devices)), auth).await
    }

    /// Direct exercise of the validation + store behaviour — the
    /// AuthenticatedUser extractor is a bearer-token-driven trait, so
    /// the cheapest unit-level coverage walks the same paths the
    /// handler does after auth resolves.

    #[tokio::test]
    async fn get_defaults_to_empty_when_nothing_stored() {
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();

        let prefs = store.get(user).await.unwrap();
        assert!(prefs.theme.is_none());
    }

    #[tokio::test]
    async fn put_round_trips_valid_theme() {
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();

        for theme in ALLOWED_THEMES {
            let prefs = UserPreferences {
                theme: Some((*theme).to_string()),
                debug_logging: None,
                auto_update_check: None,
                release_channel: None,
                api_url: None,
                remote_sync: None,
                kb_view: None,
                kb_units: None,
                timezone: None,
                theme_wave_speed: None,
            };
            store.put(user, &prefs).await.unwrap();
            let got = store.get(user).await.unwrap();
            assert_eq!(got.theme.as_deref(), Some(*theme));
        }
    }

    #[tokio::test]
    async fn invalid_theme_is_rejected_by_allowlist() {
        // The handler rejects unknown themes with 400 `invalid_theme`
        // before ever touching the store. Recreate the gate inline so
        // this test is hermetic (no axum runtime needed).
        let bad_themes = ["", "STANTON", "Stanton", "microtech", "pyrö", " "];
        for t in bad_themes {
            assert!(
                !ALLOWED_THEMES.contains(&t),
                "{t:?} unexpectedly in allowlist"
            );
        }

        // And confirm the four real themes do pass.
        for t in ["stanton", "pyro", "terra", "nyx"] {
            assert!(ALLOWED_THEMES.contains(&t), "{t:?} missing from allowlist");
        }
    }

    #[tokio::test]
    async fn empty_preferences_put_is_a_noop() {
        // A PUT with no fields set must round-trip as a no-op — the
        // stored value is unchanged. This is the sparse-merge contract
        // documented in preferences_routes.rs::put.
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();

        store
            .put(
                user,
                &UserPreferences {
                    theme: Some("pyro".into()),
                    ..UserPreferences::default()
                },
            )
            .await
            .unwrap();
        store.put(user, &UserPreferences::default()).await.unwrap();

        let got = store.get(user).await.unwrap();
        assert_eq!(got.theme.as_deref(), Some("pyro"));
    }

    #[tokio::test]
    async fn invalid_release_channel_rejected() {
        // `ReleaseChannel` enum lives in the updater module; allowed
        // values today are `live`, `beta`. Anything else → 400.
        for bad in ["", "LIVE", "Pyro", "stable", "production"] {
            assert!(
                !ALLOWED_RELEASE_CHANNELS.contains(&bad),
                "{bad:?} unexpectedly in allowlist"
            );
        }
    }

    #[tokio::test]
    async fn interval_clamps_match_documented_ranges() {
        // priority: 1..=60, bulk: 5..=3600, batch_size: 1..=5000
        assert!(!is_valid_priority_interval(0));
        assert!(is_valid_priority_interval(1));
        assert!(is_valid_priority_interval(60));
        assert!(!is_valid_priority_interval(61));

        assert!(!is_valid_bulk_interval(4));
        assert!(is_valid_bulk_interval(5));
        assert!(is_valid_bulk_interval(3600));
        assert!(!is_valid_bulk_interval(3601));

        assert!(!is_valid_batch_size(0));
        assert!(is_valid_batch_size(1));
        assert!(is_valid_batch_size(5000));
        assert!(!is_valid_batch_size(5001));
    }

    #[tokio::test]
    async fn api_url_validation_accepts_https_and_rejects_garbage() {
        assert!(is_valid_api_url("https://api.example.com"));
        assert!(is_valid_api_url("http://localhost:8080"));
        assert!(!is_valid_api_url(""));
        assert!(!is_valid_api_url("not a url"));
        assert!(!is_valid_api_url(&"x".repeat(257)));
    }

    #[tokio::test]
    async fn device_token_with_sync_enabled_can_read_preferences() {
        // Set up: a device whose sync_enabled = true. The handler
        // looks up the device row via the device_id claim; with
        // sync_enabled = true it should proceed exactly like a user
        // token.
        let prefs_store = Arc::new(MemoryPreferencesStore::new());
        let devices = Arc::new(crate::devices::test_support::MemoryDeviceStore::new());
        let user_id = Uuid::new_v4();
        let device_id = devices
            .seed_paired_device(user_id, "Daisy's PC")
            .await
            .unwrap();
        devices
            .set_sync_enabled(user_id, device_id, true)
            .await
            .unwrap();

        prefs_store
            .put(
                user_id,
                &UserPreferences {
                    theme: Some("pyro".into()),
                    ..UserPreferences::default()
                },
            )
            .await
            .unwrap();

        let auth = make_device_token_auth(user_id, device_id);
        let resp = get_handler_inner(prefs_store.clone(), devices.clone(), auth).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn device_token_with_sync_disabled_returns_403() {
        let prefs_store = Arc::new(MemoryPreferencesStore::new());
        let devices = Arc::new(crate::devices::test_support::MemoryDeviceStore::new());
        let user_id = Uuid::new_v4();
        let device_id = devices
            .seed_paired_device(user_id, "Daisy's PC")
            .await
            .unwrap();
        // sync_enabled defaults to false from seed_paired_device.

        let auth = make_device_token_auth(user_id, device_id);
        let resp = get_handler_inner(prefs_store, devices, auth).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let body_json: ApiErrorBody = serde_json::from_slice(&body).unwrap();
        assert_eq!(body_json.error, "device_sync_disabled");
    }

    #[tokio::test]
    async fn user_token_always_allowed() {
        let prefs_store = Arc::new(MemoryPreferencesStore::new());
        let devices = Arc::new(crate::devices::test_support::MemoryDeviceStore::new());
        let user_id = Uuid::new_v4();
        let auth = make_user_token_auth(user_id);
        let resp = get_handler_inner(prefs_store, devices, auth).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn kb_view_and_units_allowlists() {
        assert!(ALLOWED_KB_VIEWS.contains(&"visual"));
        assert!(ALLOWED_KB_VIEWS.contains(&"compact"));
        assert!(!ALLOWED_KB_VIEWS.contains(&"fancy"));
        assert!(ALLOWED_KB_UNITS.contains(&"metric"));
        assert!(ALLOWED_KB_UNITS.contains(&"imperial"));
        assert!(!ALLOWED_KB_UNITS.contains(&"furlongs"));
    }

    #[tokio::test]
    async fn theme_wave_speed_allowlist() {
        for speed in ["off", "slow", "normal", "fast"] {
            assert!(
                ALLOWED_WAVE_SPEEDS.contains(&speed),
                "{speed:?} missing from allowlist"
            );
        }
        assert!(!ALLOWED_WAVE_SPEEDS.contains(&"ludicrous"));
        assert!(!ALLOWED_WAVE_SPEEDS.contains(&""));
    }

    #[tokio::test]
    async fn put_round_trips_valid_wave_speed() {
        let store = MemoryPreferencesStore::new();
        let user = Uuid::new_v4();

        for speed in ALLOWED_WAVE_SPEEDS {
            let prefs = UserPreferences {
                theme_wave_speed: Some((*speed).to_string()),
                ..UserPreferences::default()
            };
            store.put(user, &prefs).await.unwrap();
            let got = store.get(user).await.unwrap();
            assert_eq!(got.theme_wave_speed.as_deref(), Some(*speed));
        }
    }
}
