//! Apply server-fetched UserPreferences onto the tray's local
//! Config. Touches only the sync-eligible fields; never
//! `gamelog_path`, `rsi_cookie`, `access_token`, `claimed_handle`,
//! or `sync_with_cloud` itself. Returns true when any field
//! actually changed — callers use that to decide whether to
//! re-persist and emit `config-changed`.
//!
//! Single source of truth for "what counts as sync-eligible" on the
//! tray side — mirrors the server's allowlist in
//! `preferences_routes.rs`. When a new sync-eligible field is added
//! later, the change lands here and in the route's validator;
//! nowhere else.

use crate::config::{Config, ReleaseChannel, Theme};
use starstats_core::wire::UserPreferences;

pub fn apply_remote_prefs(config: &mut Config, prefs: &UserPreferences) -> bool {
    let mut changed = false;

    if let Some(theme_str) = prefs.theme.as_deref() {
        if let Some(parsed) = parse_theme(theme_str) {
            if config.theme != parsed {
                config.theme = parsed;
                changed = true;
            }
        }
    }

    if let Some(v) = prefs.debug_logging {
        if config.debug_logging != v {
            config.debug_logging = v;
            changed = true;
        }
    }

    if let Some(v) = prefs.auto_update_check {
        if config.auto_update_check != v {
            config.auto_update_check = v;
            changed = true;
        }
    }

    if let Some(ch_str) = prefs.release_channel.as_deref() {
        if let Some(parsed) = parse_release_channel(ch_str) {
            if config.release_channel != parsed {
                config.release_channel = parsed;
                changed = true;
            }
        }
    }

    if let Some(url) = prefs.api_url.as_deref() {
        if config.remote_sync.api_url.as_deref() != Some(url) {
            config.remote_sync.api_url = Some(url.to_string());
            changed = true;
        }
    }

    if let Some(speed) = prefs.theme_wave_speed.as_deref() {
        if is_valid_wave_speed(speed) && config.theme_wave_speed != speed {
            config.theme_wave_speed = speed.to_string();
            changed = true;
        }
    }

    if let Some(rs) = &prefs.remote_sync {
        if let Some(v) = rs.enabled {
            if config.remote_sync.enabled != v {
                config.remote_sync.enabled = v;
                changed = true;
            }
        }
        if let Some(v) = rs.priority_interval_secs {
            let v_u64 = v as u64;
            if config.remote_sync.priority_interval_secs != v_u64 {
                config.remote_sync.priority_interval_secs = v_u64;
                changed = true;
            }
        }
        if let Some(v) = rs.interval_secs {
            let v_u64 = v as u64;
            if config.remote_sync.interval_secs != v_u64 {
                config.remote_sync.interval_secs = v_u64;
                changed = true;
            }
        }
        if let Some(v) = rs.batch_size {
            let v_usize = v as usize;
            if config.remote_sync.batch_size != v_usize {
                config.remote_sync.batch_size = v_usize;
                changed = true;
            }
        }
    }

    changed
}

/// Build a UserPreferences payload from the sync-eligible subset of
/// Config. Used when the tray seeds the server on first opt-in OR
/// writes through on Save.
pub fn snapshot_for_remote(config: &Config) -> UserPreferences {
    use starstats_core::wire::RemoteSyncPrefs;
    UserPreferences {
        theme: Some(config.theme.as_str().to_string()),
        debug_logging: Some(config.debug_logging),
        auto_update_check: Some(config.auto_update_check),
        release_channel: Some(config.release_channel.as_str().to_string()),
        api_url: config.remote_sync.api_url.clone(),
        remote_sync: Some(RemoteSyncPrefs {
            enabled: Some(config.remote_sync.enabled),
            priority_interval_secs: Some(config.remote_sync.priority_interval_secs as u32),
            interval_secs: Some(config.remote_sync.interval_secs as u32),
            batch_size: Some(config.remote_sync.batch_size as u32),
        }),
        // KB view preferences are web-only; the tray neither owns nor
        // mirrors them. `None` + serde-skip means this sync write never
        // touches the user's stored kb_view/kb_units (server sparse-merge
        // leaves absent fields alone).
        kb_view: None,
        kb_units: None,
        // Same reasoning as kb_*: the timezone is set on the web (which can
        // detect it from the browser) and the tray neither owns nor mirrors
        // it. `None` + serde-skip means a tray sync write never clears the
        // zone the player chose — the server sparse-merge leaves absent
        // fields alone.
        timezone: None,
        // M6: the tray now has its own theme-switch wave (mirroring the
        // web animation), so a synced user's speed preference follows
        // them across devices the same way `theme` does.
        theme_wave_speed: Some(config.theme_wave_speed.clone()),
    }
}

fn parse_theme(s: &str) -> Option<Theme> {
    match s {
        "stanton" => Some(Theme::Stanton),
        "pyro" => Some(Theme::Pyro),
        "terra" => Some(Theme::Terra),
        "nyx" => Some(Theme::Nyx),
        _ => None,
    }
}

/// Mirrors the server's `ALLOWED_WAVE_SPEEDS`
/// (`preferences_routes.rs` / `appearance_routes.rs`) and the
/// TS-side `WAVE_SPEEDS` (`apps/tray-ui/src/lib/wave-speed.ts`,
/// `apps/web/src/lib/wave-speed.ts`). An unrecognised value from the
/// server is ignored rather than accepted verbatim, same as
/// `parse_theme` / `parse_release_channel` above.
fn is_valid_wave_speed(s: &str) -> bool {
    matches!(s, "off" | "slow" | "normal" | "fast")
}

fn parse_release_channel(s: &str) -> Option<ReleaseChannel> {
    match s {
        "alpha" => Some(ReleaseChannel::Alpha),
        "beta" => Some(ReleaseChannel::Beta),
        "rc" => Some(ReleaseChannel::Rc),
        "live" => Some(ReleaseChannel::Live),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starstats_core::wire::RemoteSyncPrefs;

    #[test]
    fn applies_theme_change() {
        let mut cfg = Config {
            theme: Theme::Stanton,
            ..Config::default()
        };
        let prefs = UserPreferences {
            theme: Some("nyx".into()),
            ..UserPreferences::default()
        };
        assert!(apply_remote_prefs(&mut cfg, &prefs));
        assert_eq!(cfg.theme, Theme::Nyx);
    }

    #[test]
    fn returns_false_when_nothing_changes() {
        let mut cfg = Config {
            theme: Theme::Stanton,
            ..Config::default()
        };
        let prefs = UserPreferences {
            theme: Some("stanton".into()),
            ..UserPreferences::default()
        };
        assert!(!apply_remote_prefs(&mut cfg, &prefs));
    }

    #[test]
    fn ignores_unknown_theme() {
        let mut cfg = Config {
            theme: Theme::Pyro,
            ..Config::default()
        };
        let prefs = UserPreferences {
            theme: Some("magenta".into()),
            ..UserPreferences::default()
        };
        assert!(!apply_remote_prefs(&mut cfg, &prefs));
        assert_eq!(cfg.theme, Theme::Pyro);
    }

    #[test]
    fn leaves_local_only_fields_alone() {
        let cfg = Config {
            gamelog_path: Some(std::path::PathBuf::from("/local/path")),
            client_anon_id: Some("anon-xyz".into()),
            ..Config::default()
        };
        let original_path = cfg.gamelog_path.clone();
        let original_anon = cfg.client_anon_id.clone();
        let mut cfg = cfg;
        let prefs = UserPreferences {
            theme: Some("terra".into()),
            ..UserPreferences::default()
        };
        apply_remote_prefs(&mut cfg, &prefs);
        assert_eq!(cfg.gamelog_path, original_path);
        assert_eq!(cfg.client_anon_id, original_anon);
    }

    #[test]
    fn merges_nested_remote_sync_fields() {
        let mut cfg = Config::default();
        cfg.remote_sync.interval_secs = 60;
        cfg.remote_sync.batch_size = 200;
        let prefs = UserPreferences {
            remote_sync: Some(RemoteSyncPrefs {
                batch_size: Some(500),
                ..RemoteSyncPrefs::default()
            }),
            ..UserPreferences::default()
        };
        assert!(apply_remote_prefs(&mut cfg, &prefs));
        assert_eq!(cfg.remote_sync.interval_secs, 60);
        assert_eq!(cfg.remote_sync.batch_size, 500);
    }

    #[test]
    fn applies_wave_speed_change() {
        let mut cfg = Config {
            theme_wave_speed: "normal".to_string(),
            ..Config::default()
        };
        let prefs = UserPreferences {
            theme_wave_speed: Some("fast".into()),
            ..UserPreferences::default()
        };
        assert!(apply_remote_prefs(&mut cfg, &prefs));
        assert_eq!(cfg.theme_wave_speed, "fast");
    }

    #[test]
    fn ignores_unknown_wave_speed() {
        let mut cfg = Config {
            theme_wave_speed: "normal".to_string(),
            ..Config::default()
        };
        let prefs = UserPreferences {
            theme_wave_speed: Some("ludicrous".into()),
            ..UserPreferences::default()
        };
        assert!(!apply_remote_prefs(&mut cfg, &prefs));
        assert_eq!(cfg.theme_wave_speed, "normal");
    }

    #[test]
    fn snapshot_includes_wave_speed() {
        let cfg = Config {
            theme_wave_speed: "slow".to_string(),
            ..Config::default()
        };
        let snap = snapshot_for_remote(&cfg);
        assert_eq!(snap.theme_wave_speed.as_deref(), Some("slow"));
    }

    #[test]
    fn snapshot_round_trips_via_apply() {
        use crate::config::RemoteSyncConfig;
        let cfg = Config {
            theme: Theme::Pyro,
            debug_logging: true,
            theme_wave_speed: "fast".to_string(),
            remote_sync: RemoteSyncConfig {
                batch_size: 333,
                ..RemoteSyncConfig::default()
            },
            ..Config::default()
        };
        let snap = snapshot_for_remote(&cfg);

        let mut other = Config::default();
        apply_remote_prefs(&mut other, &snap);
        assert_eq!(other.theme, Theme::Pyro);
        assert!(other.debug_logging);
        assert_eq!(other.theme_wave_speed, "fast");
        assert_eq!(other.remote_sync.batch_size, 333);
    }

    #[test]
    fn applies_all_release_channels() {
        let channels = vec![
            ("alpha", ReleaseChannel::Alpha),
            ("beta", ReleaseChannel::Beta),
            ("rc", ReleaseChannel::Rc),
            ("live", ReleaseChannel::Live),
        ];

        for (channel_str, expected_channel) in channels {
            let mut cfg = Config::default();
            // Only expect changed=true if we're actually changing from default
            let is_changing = expected_channel != ReleaseChannel::Live;
            let prefs = UserPreferences {
                release_channel: Some(channel_str.into()),
                ..UserPreferences::default()
            };
            let changed = apply_remote_prefs(&mut cfg, &prefs);
            assert_eq!(
                changed, is_changing,
                "expected changed={} when setting to {}",
                is_changing, channel_str
            );
            assert_eq!(cfg.release_channel, expected_channel);
        }
    }
}
