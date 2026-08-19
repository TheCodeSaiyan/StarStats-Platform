//! Aggregated health surface for the tray UI.
//!
//! `current_health()` is a *pure* function over a `HealthInputs`
//! snapshot — it does no I/O. The caller (`commands::get_health`) is
//! responsible for assembling the snapshot from `AppState`, `Config`,
//! the secret store, and `sysinfo`. Keeping derivation pure lets us
//! exhaustively unit-test every check from in-memory fixtures.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warn,
    Info,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HealthId {
    GamelogMissing,
    ApiUrlMissing,
    PairMissing,
    AuthLost,
    CookieMissing,
    SyncFailing,
    HangarSkip,
    EmailUnverified,
    GameLogStale,
    UpdateAvailable,
    DiskFreeLow,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SettingsField {
    GamelogPath,
    ApiUrl,
    PairingCode,
    RsiCookie,
    Updates,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HealthAction {
    GoToSettings { field: SettingsField },
    RetrySync,
    RefreshHangar,
    OpenUrl { url: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "id", rename_all = "snake_case")]
pub enum HealthParams {
    GamelogMissing,
    ApiUrlMissing,
    PairMissing,
    AuthLost,
    CookieMissing,
    SyncFailing {
        last_error: String,
        attempts_since_success: u32,
    },
    HangarSkip {
        reason: String,
        since: String,
    },
    EmailUnverified,
    GameLogStale {
        last_event_at: String,
    },
    UpdateAvailable {
        version: String,
    },
    DiskFreeLow {
        free_bytes: u64,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthItem {
    pub id: HealthId,
    pub severity: Severity,
    pub params: HealthParams,
    pub action: Option<HealthAction>,
    pub dismissible: bool,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DismissedHealth {
    pub id: HealthId,
    pub fingerprint: String,
    pub dismissed_at: DateTime<Utc>,
}

/// Read-only snapshot of every state slice `current_health` derives
/// from. Constructed by `commands::get_health` from `AppState`,
/// `Config`, the secret store, and `sysinfo`. Keeping it a separate
/// struct keeps the derivation pure and testable.
#[derive(Debug, Clone)]
pub struct HealthInputs {
    pub now: DateTime<Utc>,
    pub gamelog_discovered_count: usize,
    pub gamelog_override_set: bool,
    pub remote_sync_enabled: bool,
    pub api_url: Option<String>,
    pub access_token: Option<String>,
    pub web_origin: Option<String>,
    pub auth_lost: bool,
    pub email_verified: Option<bool>,
    pub cookie_configured: bool,
    pub sync_last_error: Option<String>,
    pub sync_attempts_since_success: u32,
    pub hangar_last_attempt_at: Option<DateTime<Utc>>,
    pub hangar_last_success_at: Option<DateTime<Utc>>,
    pub hangar_last_skip_reason: Option<String>,
    pub tail_current_path: Option<String>,
    pub tail_last_event_at: Option<DateTime<Utc>>,
    pub sc_process_running: bool,
    pub disk_free_bytes: Option<u64>,
    pub update_available_version: Option<String>,
    pub dismissed: Vec<DismissedHealth>,
}

const GAME_LOG_STALE_MIN: i64 = 30;
const DISK_FREE_LOW_THRESHOLD: u64 = 1_073_741_824; // 1 GiB

/// Derive the ordered list of HealthItems from a snapshot. Pure — no I/O.
pub fn current_health(inputs: &HealthInputs) -> Vec<HealthItem> {
    let mut items: Vec<HealthItem> = Vec::new();

    if inputs.gamelog_discovered_count == 0 && !inputs.gamelog_override_set {
        items.push(item(
            HealthId::GamelogMissing,
            Severity::Warn,
            HealthParams::GamelogMissing,
            Some(HealthAction::GoToSettings {
                field: SettingsField::GamelogPath,
            }),
        ));
    }

    if inputs.remote_sync_enabled && inputs.api_url.is_none() {
        items.push(item(
            HealthId::ApiUrlMissing,
            Severity::Warn,
            HealthParams::ApiUrlMissing,
            Some(HealthAction::GoToSettings {
                field: SettingsField::ApiUrl,
            }),
        ));
    }

    if inputs.remote_sync_enabled && inputs.api_url.is_some() && inputs.access_token.is_none() {
        items.push(item(
            HealthId::PairMissing,
            Severity::Warn,
            HealthParams::PairMissing,
            Some(HealthAction::GoToSettings {
                field: SettingsField::PairingCode,
            }),
        ));
    }

    if inputs.auth_lost {
        items.push(item(
            HealthId::AuthLost,
            Severity::Error,
            HealthParams::AuthLost,
            Some(HealthAction::GoToSettings {
                field: SettingsField::PairingCode,
            }),
        ));
    }

    // CookieMissing — only when paired AND a hangar attempt has happened.
    let paired = inputs.api_url.is_some() && inputs.access_token.is_some();
    let hangar_engaged = inputs.hangar_last_attempt_at.is_some();
    if paired && !inputs.cookie_configured && hangar_engaged {
        items.push(item(
            HealthId::CookieMissing,
            Severity::Warn,
            HealthParams::CookieMissing,
            Some(HealthAction::GoToSettings {
                field: SettingsField::RsiCookie,
            }),
        ));
    }

    // SyncFailing — suppressed by AuthLost (same root cause).
    if inputs.sync_last_error.is_some() && !inputs.auth_lost {
        let last_error = inputs.sync_last_error.clone().unwrap();
        items.push(item(
            HealthId::SyncFailing,
            Severity::Error,
            HealthParams::SyncFailing {
                last_error,
                attempts_since_success: inputs.sync_attempts_since_success,
            },
            Some(HealthAction::RetrySync),
        ));
    }

    // HangarSkip — only when never-succeeded.
    if inputs.hangar_last_skip_reason.is_some() && inputs.hangar_last_success_at.is_none() {
        let reason = inputs.hangar_last_skip_reason.clone().unwrap();
        let since = inputs
            .hangar_last_attempt_at
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| inputs.now.to_rfc3339());
        items.push(item(
            HealthId::HangarSkip,
            Severity::Warn,
            HealthParams::HangarSkip { reason, since },
            Some(HealthAction::RefreshHangar),
        ));
    }

    if inputs.email_verified == Some(false) {
        if let Some(origin) = &inputs.web_origin {
            if origin.starts_with("http://") || origin.starts_with("https://") {
                let url = format!("{}/verify-email", origin.trim_end_matches('/'));
                items.push(item(
                    HealthId::EmailUnverified,
                    Severity::Warn,
                    HealthParams::EmailUnverified,
                    Some(HealthAction::OpenUrl { url }),
                ));
            }
        }
    }

    // GameLogStale — only when SC is running AND tail open AND quiet > 30 min.
    if inputs.sc_process_running && inputs.tail_current_path.is_some() {
        if let Some(last) = inputs.tail_last_event_at {
            let age_min = (inputs.now - last).num_minutes();
            if age_min >= GAME_LOG_STALE_MIN {
                items.push(item(
                    HealthId::GameLogStale,
                    Severity::Warn,
                    HealthParams::GameLogStale {
                        last_event_at: last.to_rfc3339(),
                    },
                    None,
                ));
            }
        }
    }

    if let Some(version) = &inputs.update_available_version {
        items.push(item(
            HealthId::UpdateAvailable,
            Severity::Info,
            HealthParams::UpdateAvailable {
                version: version.clone(),
            },
            Some(HealthAction::GoToSettings {
                field: SettingsField::Updates,
            }),
        ));
    }

    if let Some(free) = inputs.disk_free_bytes {
        if free < DISK_FREE_LOW_THRESHOLD {
            items.push(item(
                HealthId::DiskFreeLow,
                Severity::Warn,
                HealthParams::DiskFreeLow { free_bytes: free },
                None,
            ));
        }
    }

    items.retain(|i| !is_dismissed(i, &inputs.dismissed));
    items.sort_by_key(|i| (severity_order(i.severity), id_order(i.id)));
    items
}

fn item(
    id: HealthId,
    severity: Severity,
    params: HealthParams,
    action: Option<HealthAction>,
) -> HealthItem {
    let fingerprint = fingerprint(id, &params);
    HealthItem {
        id,
        severity,
        params,
        action,
        dismissible: dismissible_for(severity),
        fingerprint,
    }
}

fn dismissible_for(severity: Severity) -> bool {
    matches!(severity, Severity::Warn | Severity::Info)
}

fn fingerprint(_id: HealthId, params: &HealthParams) -> String {
    // Canonical JSON serialisation of params — stable across runs
    // and human-readable in the persisted config. The id is already
    // carried by the surrounding DismissedHealth.id, and HealthParams
    // is `#[serde(tag = "id")]` so the id is also present in the
    // serialised string itself — including a separate copy in the
    // tuple would double-encode it in every persisted entry forever.
    serde_json::to_string(params).unwrap_or_else(|_| String::from("invalid"))
}

fn is_dismissed(item: &HealthItem, dismissed: &[DismissedHealth]) -> bool {
    dismissed
        .iter()
        .any(|d| d.id == item.id && d.fingerprint == item.fingerprint)
}

fn severity_order(s: Severity) -> u8 {
    match s {
        Severity::Error => 0,
        Severity::Warn => 1,
        Severity::Info => 2,
    }
}

fn id_order(id: HealthId) -> u8 {
    match id {
        HealthId::GamelogMissing => 0,
        HealthId::ApiUrlMissing => 1,
        HealthId::PairMissing => 2,
        HealthId::AuthLost => 3,
        HealthId::CookieMissing => 4,
        HealthId::SyncFailing => 5,
        HealthId::HangarSkip => 6,
        HealthId::EmailUnverified => 7,
        HealthId::GameLogStale => 8,
        HealthId::UpdateAvailable => 9,
        HealthId::DiskFreeLow => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_inputs() -> HealthInputs {
        HealthInputs {
            now: chrono::Utc::now(),
            gamelog_discovered_count: 0,
            gamelog_override_set: false,
            remote_sync_enabled: false,
            api_url: None,
            access_token: None,
            web_origin: None,
            auth_lost: false,
            email_verified: None,
            cookie_configured: false,
            sync_last_error: None,
            sync_attempts_since_success: 0,
            hangar_last_attempt_at: None,
            hangar_last_success_at: None,
            hangar_last_skip_reason: None,
            tail_current_path: None,
            tail_last_event_at: None,
            sc_process_running: false,
            disk_free_bytes: None,
            update_available_version: None,
            dismissed: Vec::new(),
        }
    }

    #[test]
    fn severity_serialises_snake_case() {
        let s = serde_json::to_string(&Severity::Error).unwrap();
        assert_eq!(s, "\"error\"");
    }

    #[test]
    fn health_action_uses_kind_tag() {
        let action = HealthAction::GoToSettings {
            field: SettingsField::ApiUrl,
        };
        let s = serde_json::to_string(&action).unwrap();
        assert!(s.contains("\"kind\":\"go_to_settings\""));
        assert!(s.contains("\"field\":\"api_url\""));
    }

    #[test]
    fn fingerprint_is_stable_for_same_params() {
        let a = fingerprint(
            HealthId::SyncFailing,
            &HealthParams::SyncFailing {
                last_error: "502 Bad Gateway".into(),
                attempts_since_success: 3,
            },
        );
        let b = fingerprint(
            HealthId::SyncFailing,
            &HealthParams::SyncFailing {
                last_error: "502 Bad Gateway".into(),
                attempts_since_success: 3,
            },
        );
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_differs_when_params_change() {
        let a = fingerprint(
            HealthId::SyncFailing,
            &HealthParams::SyncFailing {
                last_error: "502".into(),
                attempts_since_success: 1,
            },
        );
        let b = fingerprint(
            HealthId::SyncFailing,
            &HealthParams::SyncFailing {
                last_error: "401".into(),
                attempts_since_success: 1,
            },
        );
        assert_ne!(a, b);
    }

    #[test]
    fn gamelog_missing_fires_when_no_logs_and_no_override() {
        let inputs = empty_inputs();
        let items = current_health(&inputs);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, HealthId::GamelogMissing);
        assert_eq!(items[0].severity, Severity::Warn);
        assert!(items[0].dismissible);
        assert_eq!(
            items[0].action,
            Some(HealthAction::GoToSettings {
                field: SettingsField::GamelogPath
            })
        );
    }

    #[test]
    fn gamelog_missing_does_not_fire_when_override_set() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::GamelogMissing));
    }

    #[test]
    fn gamelog_missing_does_not_fire_when_logs_discovered() {
        let mut inputs = empty_inputs();
        inputs.gamelog_discovered_count = 1;
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::GamelogMissing));
    }

    #[test]
    fn api_url_missing_fires_when_remote_sync_enabled_and_url_unset() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.remote_sync_enabled = true;
        let items = current_health(&inputs);
        assert!(items.iter().any(|i| i.id == HealthId::ApiUrlMissing));
        let item = items
            .iter()
            .find(|i| i.id == HealthId::ApiUrlMissing)
            .unwrap();
        assert_eq!(item.severity, Severity::Warn);
        assert_eq!(
            item.action,
            Some(HealthAction::GoToSettings {
                field: SettingsField::ApiUrl
            })
        );
    }

    #[test]
    fn api_url_missing_silent_when_remote_sync_disabled() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::ApiUrlMissing));
    }

    #[test]
    fn pair_missing_fires_when_url_set_but_no_token() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.remote_sync_enabled = true;
        inputs.api_url = Some("https://api.example".into());
        let items = current_health(&inputs);
        assert!(items.iter().any(|i| i.id == HealthId::PairMissing));
    }

    #[test]
    fn pair_missing_silent_without_api_url() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.remote_sync_enabled = true;
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::PairMissing));
    }

    #[test]
    fn auth_lost_fires_as_error_when_flag_set() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.auth_lost = true;
        let items = current_health(&inputs);
        let item = items.iter().find(|i| i.id == HealthId::AuthLost).unwrap();
        assert_eq!(item.severity, Severity::Error);
        assert!(!item.dismissible);
    }

    #[test]
    fn sync_failing_fires_when_last_error_present_and_not_auth_lost() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.sync_last_error = Some("502 Bad Gateway".into());
        inputs.sync_attempts_since_success = 4;
        let items = current_health(&inputs);
        let item = items
            .iter()
            .find(|i| i.id == HealthId::SyncFailing)
            .unwrap();
        assert_eq!(item.severity, Severity::Error);
        match &item.params {
            HealthParams::SyncFailing {
                last_error,
                attempts_since_success,
            } => {
                assert_eq!(last_error, "502 Bad Gateway");
                assert_eq!(*attempts_since_success, 4);
            }
            _ => panic!("wrong params variant"),
        }
        assert_eq!(item.action, Some(HealthAction::RetrySync));
    }

    #[test]
    fn sync_failing_suppressed_when_auth_lost() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.auth_lost = true;
        inputs.sync_last_error = Some("401 Unauthorized".into());
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::SyncFailing));
        assert!(items.iter().any(|i| i.id == HealthId::AuthLost));
    }

    #[test]
    fn hangar_skip_fires_when_never_succeeded() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.hangar_last_skip_reason = Some("cookie missing".into());
        inputs.hangar_last_attempt_at = Some(inputs.now);
        let items = current_health(&inputs);
        let item = items.iter().find(|i| i.id == HealthId::HangarSkip).unwrap();
        assert_eq!(item.severity, Severity::Warn);
        assert_eq!(item.action, Some(HealthAction::RefreshHangar));
    }

    #[test]
    fn hangar_skip_silent_after_a_prior_success() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.hangar_last_skip_reason = Some("rate limited".into());
        inputs.hangar_last_success_at = Some(inputs.now - chrono::Duration::hours(2));
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::HangarSkip));
    }

    #[test]
    fn cookie_missing_fires_when_paired_and_hangar_attempted() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.remote_sync_enabled = true;
        inputs.api_url = Some("https://api.example".into());
        inputs.access_token = Some("tok".into());
        inputs.cookie_configured = false;
        inputs.hangar_last_attempt_at = Some(inputs.now);
        let items = current_health(&inputs);
        let item = items
            .iter()
            .find(|i| i.id == HealthId::CookieMissing)
            .unwrap();
        assert_eq!(item.severity, Severity::Warn);
        assert!(item.dismissible);
    }

    #[test]
    fn cookie_missing_silent_without_hangar_attempt() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.remote_sync_enabled = true;
        inputs.api_url = Some("https://api.example".into());
        inputs.access_token = Some("tok".into());
        inputs.cookie_configured = false;
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::CookieMissing));
    }

    #[test]
    fn email_unverified_fires_when_flag_false_and_web_origin_set() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.email_verified = Some(false);
        inputs.web_origin = Some("https://app.example".into());
        let items = current_health(&inputs);
        let item = items
            .iter()
            .find(|i| i.id == HealthId::EmailUnverified)
            .unwrap();
        assert_eq!(item.severity, Severity::Warn);
        match &item.action {
            Some(HealthAction::OpenUrl { url }) => {
                assert_eq!(url, "https://app.example/verify-email")
            }
            _ => panic!("expected OpenUrl action"),
        }
    }

    #[test]
    fn email_unverified_silent_without_web_origin() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.email_verified = Some(false);
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::EmailUnverified));
    }

    #[test]
    fn game_log_stale_fires_when_sc_running_and_30min_quiet() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.sc_process_running = true;
        inputs.tail_current_path = Some("C:/SC/Game.log".into());
        inputs.tail_last_event_at = Some(inputs.now - chrono::Duration::minutes(31));
        let items = current_health(&inputs);
        let item = items
            .iter()
            .find(|i| i.id == HealthId::GameLogStale)
            .unwrap();
        assert_eq!(item.severity, Severity::Warn);
        assert!(item.action.is_none());
    }

    #[test]
    fn game_log_stale_silent_when_sc_not_running() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.tail_current_path = Some("C:/SC/Game.log".into());
        inputs.tail_last_event_at = Some(inputs.now - chrono::Duration::hours(2));
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::GameLogStale));
    }

    #[test]
    fn game_log_stale_silent_when_recent_event() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.sc_process_running = true;
        inputs.tail_current_path = Some("C:/SC/Game.log".into());
        inputs.tail_last_event_at = Some(inputs.now - chrono::Duration::minutes(5));
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::GameLogStale));
    }

    #[test]
    fn update_available_fires_when_version_present() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.update_available_version = Some("0.4.1-beta".into());
        let items = current_health(&inputs);
        let item = items
            .iter()
            .find(|i| i.id == HealthId::UpdateAvailable)
            .unwrap();
        assert_eq!(item.severity, Severity::Info);
        match &item.params {
            HealthParams::UpdateAvailable { version } => assert_eq!(version, "0.4.1-beta"),
            _ => panic!("wrong params"),
        }
        assert_eq!(
            item.action,
            Some(HealthAction::GoToSettings {
                field: SettingsField::Updates
            })
        );
    }

    #[test]
    fn disk_free_low_fires_below_one_gib() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.disk_free_bytes = Some(500 * 1024 * 1024);
        let items = current_health(&inputs);
        let item = items
            .iter()
            .find(|i| i.id == HealthId::DiskFreeLow)
            .unwrap();
        assert_eq!(item.severity, Severity::Warn);
        assert!(item.action.is_none());
    }

    #[test]
    fn disk_free_low_silent_above_one_gib() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.disk_free_bytes = Some(2 * DISK_FREE_LOW_THRESHOLD);
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::DiskFreeLow));
    }

    #[test]
    fn disk_free_low_silent_when_unknown() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.disk_free_bytes = None;
        let items = current_health(&inputs);
        assert!(items.iter().all(|i| i.id != HealthId::DiskFreeLow));
    }

    #[test]
    fn dismissed_item_does_not_appear() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.sync_last_error = Some("502 BG".into());
        let raw = current_health(&inputs);
        let target = raw
            .iter()
            .find(|i| i.id == HealthId::SyncFailing)
            .unwrap()
            .clone();
        inputs.dismissed.push(DismissedHealth {
            id: target.id,
            fingerprint: target.fingerprint.clone(),
            dismissed_at: chrono::Utc::now(),
        });
        let after = current_health(&inputs);
        assert!(after.iter().all(|i| i.id != HealthId::SyncFailing));
    }

    #[test]
    fn dismissed_item_reemerges_when_params_change() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.sync_last_error = Some("502 BG".into());
        let raw = current_health(&inputs);
        let target = raw
            .iter()
            .find(|i| i.id == HealthId::SyncFailing)
            .unwrap()
            .clone();
        inputs.dismissed.push(DismissedHealth {
            id: target.id,
            fingerprint: target.fingerprint.clone(),
            dismissed_at: chrono::Utc::now(),
        });
        inputs.sync_last_error = Some("401 Unauthorized".into());
        let after = current_health(&inputs);
        assert!(after.iter().any(|i| i.id == HealthId::SyncFailing));
    }

    #[test]
    fn ordering_puts_errors_before_warns_before_infos() {
        let mut inputs = empty_inputs();
        inputs.gamelog_override_set = true;
        inputs.auth_lost = true; // Error
        inputs.update_available_version = Some("0.4.1".into()); // Info
        inputs.remote_sync_enabled = true;
        inputs.api_url = None; // Warn (ApiUrlMissing)
        let items = current_health(&inputs);
        let severities: Vec<_> = items.iter().map(|i| i.severity).collect();
        assert_eq!(severities[0], Severity::Error);
        let last = *severities.last().unwrap();
        assert_eq!(last, Severity::Info);
    }
}
