//! On-disk client configuration and per-platform paths.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Persisted user configuration. Lives at
///  - Windows: `%APPDATA%\StarStats\config.toml`
///  - Linux:   `$XDG_CONFIG_HOME/StarStats/config.toml`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Override the auto-discovered Game.log path.
    pub gamelog_path: Option<PathBuf>,
    /// Sync to the remote StarStats API server.
    pub remote_sync: RemoteSyncConfig,
    /// Web UI origin — used to deep-link the user back to the website
    /// (e.g. for email verification, "Open on web"). When unset, the
    /// effective value is derived from `remote_sync.api_url` by
    /// stripping a leading `api.` from the hostname (see
    /// `Config::effective_web_origin`), so most users don't need to
    /// configure it. Custom deployments with a non-`api.` host should
    /// set this explicitly.
    pub web_origin: Option<String>,
    /// Opt in to reporting UNCLASSIFIED LOG TAG NAMES to the server so a
    /// broken parser can be diagnosed. Off by default.
    ///
    /// Sends only the `<EventName>` shell tag of lines the parser could not
    /// classify, plus first/last sighting and a count — engine symbol names
    /// like `LandingArea_UnregisterFromExternalSystems_StowingVehicle`.
    /// NEVER log line bodies. The tags let the server correlate "this event
    /// type went dark" with "this new tag appeared the same week", which is
    /// the difference between a three-week outage and a one-glance fix.
    #[serde(default)]
    pub share_unknown_tags: bool,
    /// Automatically check for updates on startup. Defaults to true;
    /// the Updates card in Settings exposes a toggle. Disabled users
    /// can still trigger a manual check via the same card.
    #[serde(default = "default_auto_update_check")]
    pub auto_update_check: bool,
    /// Which release channel to track. Drives the updater endpoint —
    /// each channel has its own manifest at
    /// `release-manifests/tray-<channel>.json` on the main branch.
    /// Defaults to [`ReleaseChannel::Live`] regardless of the build's
    /// own version suffix: opting into a pre-release channel is an
    /// explicit user choice via the Settings dropdown. The build's
    /// channel (parsed via [`ReleaseChannel::from_version`]) is only
    /// relevant when the user explicitly opts into matching it.
    #[serde(default)]
    pub release_channel: ReleaseChannel,
    /// Last build-channel value the user dismissed the channel-mismatch
    /// banner for. `None` = never dismissed. When the running binary's
    /// channel (parsed from `CARGO_PKG_VERSION`) differs from
    /// `release_channel`, the tray surfaces a banner offering to switch.
    /// "Dismiss" writes the *build* channel's `as_str()` here so future
    /// launches stay quiet — until the user upgrades into a *different*
    /// channel's build, at which point the stored value no longer matches
    /// and the banner re-appears.
    ///
    /// Stored as String (not ReleaseChannel) so we don't have to handle
    /// the deserialize-of-removed-variant edge case. Closed-vocabulary
    /// enums stored as TEXT is the project convention (see docs/ENGINEERING.md).
    #[serde(default)]
    pub channel_mismatch_ack: Option<String>,
    /// When true, the tray writes a daily-rolling `client.log` to
    /// the user data dir for diagnostics. Defaults to false to keep
    /// disk use minimal — toggle on from Settings → Updates if you
    /// need to capture logs for a bug report. The panic-only log is
    /// always written regardless of this flag.
    #[serde(default)]
    pub debug_logging: bool,
    /// Per-install opt-in for cloud sync. When true, the tray reads
    /// from and writes to `/v1/me/preferences` for the synced
    /// subset of Config (theme, debug_logging, auto_update_check,
    /// release_channel, remote_sync.*, api_url). Default false.
    /// Never synced itself — it's the gate, not the payload.
    /// See the release design notes §7.
    #[serde(default)]
    pub sync_with_cloud: bool,
    /// Visual theme applied to the tray webview. Drives the
    /// `[data-theme="..."]` attribute the design tokens scope against.
    /// Defaults to Stanton (warm amber) — the design system's canonical
    /// dark theme.
    #[serde(default)]
    pub theme: Theme,
    /// Theme-switch wave animation speed applied to the tray webview.
    /// Drives the `[data-wave-speed]` attribute
    /// `lib/theme-transition.ts` reads to resolve the sweep duration.
    /// One of `"off" | "slow" | "normal" | "fast"` — kept as a plain
    /// `String` rather than an enum to mirror the core
    /// `UserPreferences.theme_wave_speed` wire shape (validated only
    /// at the point of use, same as the web client's `WaveSpeed`
    /// guard); an unrecognised value falls back to the default
    /// TS-side. Defaults to `"normal"`, matching the server's
    /// `ALLOWED_WAVE_SPEEDS` default. `#[serde(default = ...)]`
    /// (rather than the bare `#[serde(default)]` used on `theme`)
    /// because `String::default()` is `""`, not `"normal"` — configs
    /// persisted before this field existed must still resolve to a
    /// valid speed.
    #[serde(default = "default_wave_speed")]
    pub theme_wave_speed: String,
    /// Per-user dismissal log for Health items. Permanent (no
    /// expiry); items re-emerge when the underlying params change
    /// (the fingerprint is over (id, params), not (id) alone).
    /// Only `Severity::Warn` and `Severity::Info` items are
    /// dismissible — the rule is enforced Rust-side in `health.rs`.
    #[serde(default)]
    pub dismissed_health: Vec<crate::health::DismissedHealth>,
    /// Stable per-install anonymous ID for parser submissions. The
    /// server uses `(shape_hash, client_anon_id)` as the dedupe key
    /// so repeated submissions from the same install fold into one
    /// row. Lazily generated on first call to
    /// `get_or_create_client_anon_id()` and persisted from then on.
    /// `Option` so existing config.toml files survive the upgrade
    /// without a migration.
    #[serde(default)]
    pub client_anon_id: Option<String>,
    /// Gate the v2 event-metadata pipeline (inference + unknown-line
    /// capture). **On by default** (since the parser-upgrade rollout):
    /// capture is local-only — captured lines sit in the tray's review
    /// queue and are NEVER auto-uploaded; submission stays an explicit,
    /// redaction-reviewed user action. A user who wants it off sets
    /// `parser_enable_v2_metadata = false` in config.toml (that explicit
    /// opt-out is honoured); the flag is `default = "default_true"` so a
    /// config missing the field — the existing paired base — opts in.
    #[serde(default = "default_true")]
    pub parser_enable_v2_metadata: bool,
    /// User preference for launching StarStats at system sign-in.
    /// `Some(true)` = enabled, `Some(false)` = disabled, `None` = first
    /// run (treat as opt-in: enable on first launch, set to Some(true)).
    /// Reconciliation between this field and the per-OS autostart entry
    /// lives in `main.rs::setup` — it runs once per launch and writes
    /// the OS-side state to match this preference.
    #[serde(default)]
    pub autostart_enabled: Option<bool>,
    /// Opt-in connector to a self-hosted **org platform** (the
    /// `orgplatform` companion project). When enabled, the tray forwards
    /// read-only presence [`Telemetry`](crate::org_connector) derived
    /// from already-parsed `Game.log` events to the org platform's
    /// authenticated member channel. Off by default; never synced via
    /// cloud preferences (it's a per-install, per-org link, not a
    /// portable preference). See `crate::org_connector`.
    #[serde(default)]
    pub org_connector: OrgConnectorConfig,
}

fn default_true() -> bool {
    true
}

fn default_wave_speed() -> String {
    "normal".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gamelog_path: None,
            remote_sync: RemoteSyncConfig::default(),
            web_origin: None,
            share_unknown_tags: false,
            auto_update_check: default_auto_update_check(),
            release_channel: ReleaseChannel::default(),
            channel_mismatch_ack: None,
            debug_logging: false,
            sync_with_cloud: false,
            theme: Theme::default(),
            theme_wave_speed: default_wave_speed(),
            dismissed_health: Vec::new(),
            client_anon_id: None,
            parser_enable_v2_metadata: true,
            autostart_enabled: None,
            org_connector: OrgConnectorConfig::default(),
        }
    }
}

/// Configuration for the opt-in org-platform connector
/// (`crate::org_connector`). All three fields must be set for the
/// connector to start: `enabled = true`, a `platform_url` (the org
/// platform's origin — must be `https://` or `wss://` except for a
/// loopback test host, where plaintext is allowed), and a `bearer_token`
/// (the org platform's desktop/member token).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OrgConnectorConfig {
    /// Master switch. When false, the connector never spawns.
    pub enabled: bool,
    /// Org platform origin, e.g. `https://org.example` or `wss://org.example`.
    /// Must use TLS (`https`/`wss`) for any non-loopback host — plaintext
    /// `ws://`/`http://` is only accepted for `localhost` / `127.*` / `::1`.
    /// The connector targets `/ws/member` on this host; any path is ignored.
    pub platform_url: Option<String>,
    /// Bearer token the org platform issued for this member's desktop
    /// link (the same credential the org HUD overlay uses).
    /// M-T6: never serialised — it lives in the OS keychain
    /// (`secret::ACCOUNT_ORG_BEARER`), hydrated onto this field by
    /// `config::load` and written via the `set_org_bearer` command.
    /// `#[serde(skip)]` keeps it out of both `config.toml` and the IPC emit.
    #[serde(skip)]
    pub bearer_token: Option<String>,
}

impl Config {
    /// Resolve the effective web origin for deep-link affordances
    /// (e.g. the tray's "Open on web" button).
    ///
    /// Priority order:
    /// 1. Explicit `web_origin` from config.toml — honoured verbatim.
    /// 2. Derived from `remote_sync.api_url` by stripping a leading
    ///    `api.` from the hostname (e.g. `https://api.starstats.app`
    ///    → `https://starstats.app`). The rewrite preserves scheme,
    ///    port, and path; only the host is touched.
    /// 3. `None` when neither is usable.
    ///
    /// Returning the API URL unmodified is never correct: the API
    /// subdomain serves JSON, not HTML, so `/u/<handle>` 404s. The
    /// old TS-side fallback chain (App.tsx) did exactly that — the
    /// fix is to move the resolution Rust-side so the contract is a
    /// single source of truth.
    /// Whether the v2 event-metadata pipeline (inference engine +
    /// unknown-line capture) is enabled. Drives the gating in
    /// `gamelog::ingest_one_line` — when false, the ingest path
    /// classifies and records the legacy unknown-event sample only;
    /// when true, unknown lines additionally land in the local
    /// `unknown_lines` SQLite cache for the review queue.
    pub fn v2_metadata_enabled(&self) -> bool {
        self.parser_enable_v2_metadata
    }

    pub fn effective_web_origin(&self) -> Option<String> {
        if let Some(origin) = self.web_origin.as_deref() {
            let trimmed = origin.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.trim_end_matches('/').to_string());
            }
        }
        derive_web_origin_from_api_url(self.remote_sync.api_url.as_deref()?)
    }
}

/// Best-effort `api.<rest>` → `<rest>` host rewrite. Returns `None`
/// for unparseable URLs or hosts that don't start with `api.`.
/// Preserves scheme and the host's port suffix; the path is
/// discarded (we want an origin, not a deep link).
///
/// String-parsing rather than a `url::Url` round-trip on purpose —
/// avoids pulling the `url` crate into the client just for this
/// single helper, and the input shape is well-constrained
/// (`scheme://host[:port][/...]`).
fn derive_web_origin_from_api_url(api_url: &str) -> Option<String> {
    let trimmed = api_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let (scheme, rest) = trimmed.split_once("://")?;
    if scheme.is_empty() || rest.is_empty() {
        return None;
    }
    // `rest` is `host[:port][/path][?query]`. Strip the path/query
    // first so a path segment containing `api.` can't trip the host
    // rewrite (would-be `https://example.com/api.bar` is left alone).
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let new_authority = authority.strip_prefix("api.")?;
    if new_authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{new_authority}"))
}

/// User-selectable visual theme. Each variant matches one of the four
/// `[data-theme="..."]` blocks in `starstats-tokens.css` — switching
/// themes is just a paint change (no layout reflow, no font swap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Warm amber on charcoal. The design system's default.
    Stanton,
    /// Molten coral, more aggressive accent. Dark.
    Pyro,
    /// Cool teal, clinical. Dark.
    Terra,
    /// Deep violet on warm off-white. Light.
    Nyx,
}

impl Default for Theme {
    fn default() -> Self {
        Self::Stanton
    }
}

impl Theme {
    /// Lowercase token serialised into config.toml and matched by the
    /// `[data-theme="..."]` selectors in `starstats-tokens.css`.
    /// Currently unused (serde's `rename_all = "snake_case"` produces
    /// the same string for the persistence path), but kept on the
    /// public API for callers that need the literal token without a
    /// `serde_json::to_value` round-trip.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::Stanton => "stanton",
            Theme::Pyro => "pyro",
            Theme::Terra => "terra",
            Theme::Nyx => "nyx",
        }
    }
}

/// User-selectable release channel. Each channel maps to a stable
/// manifest URL on the `main` branch; the release workflow writes the
/// generated manifest into `release-manifests/tray-<channel>.json` based
/// on the tag's pre-release suffix.
///
/// Switching channels changes which manifest the updater queries on
/// next check — no reinstall required. The Tauri updater only offers
/// a download when the manifest version is strictly greater than the
/// installed version (semver), so switching from Beta to Live while
/// running a newer prerelease will not roll back; you'll simply
/// receive nothing until Live catches up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    /// Pre-release alpha builds — `vX.Y.Z-alpha[.N]`. Retained as an
    /// opt-in channel; no longer the default channel for fresh installs
    /// (see `Default` impl).
    Alpha,
    /// Beta builds — `vX.Y.Z-beta[.N]`. The active pre-release channel
    /// post-history-scrub.
    Beta,
    /// Release candidates — `vX.Y.Z-rc[.N]`. Intended for users who
    /// want stability ahead of GA but accept the occasional regression.
    Rc,
    /// Stable releases — bare `vX.Y.Z` tags. The conservative default
    /// once the project hits 1.0; for now this channel is empty.
    Live,
}

impl Default for ReleaseChannel {
    /// Default channel for fresh installs is `Live` (stable releases),
    /// regardless of which pre-release the binary itself shipped on.
    /// Rationale: most users want stability by default — they can
    /// opt into Beta/RC/Alpha from the Settings dropdown when they
    /// want pre-release builds. Persisted user overrides (config.toml)
    /// still win over this default.
    ///
    /// (Earlier behaviour derived the default from `CARGO_PKG_VERSION`,
    /// so a `-beta` binary defaulted to Beta. That made the channel
    /// implicit and surprised users who didn't realise they were on a
    /// pre-release channel.)
    fn default() -> Self {
        Self::Live
    }
}

impl ReleaseChannel {
    /// Lowercase token used in the manifest filename and the Settings
    /// dropdown's serialised value.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReleaseChannel::Alpha => "alpha",
            ReleaseChannel::Beta => "beta",
            ReleaseChannel::Rc => "rc",
            ReleaseChannel::Live => "live",
        }
    }

    /// Map a semver string to a channel by inspecting its prerelease
    /// suffix. Anything without a recognised suffix is treated as Live
    /// (the conservative choice for unrecognised inputs).
    ///
    /// Used by `commands::get_build_release_channel` to surface the
    /// running binary's channel to the tray Settings UI, where it is
    /// compared against `Config::release_channel` to drive the
    /// channel-mismatch banner. The semver→channel mapping is the
    /// canonical place to express the build-version → channel
    /// conversion; the `Default for Config` still pins `release_channel`
    /// to `Live` regardless of build version (opting into a pre-release
    /// channel is an explicit user choice).
    pub fn from_version(v: &str) -> Self {
        let Some((_, suffix)) = v.split_once('-') else {
            return Self::Live;
        };
        // suffix may be "alpha", "alpha.1", "beta.2", "rc", etc.
        match suffix.split('.').next().unwrap_or("") {
            "alpha" => Self::Alpha,
            "beta" => Self::Beta,
            "rc" => Self::Rc,
            _ => Self::Live,
        }
    }

    /// Stable updater endpoint for this channel — points at the
    /// manifest on the main branch via raw.githubusercontent.com.
    /// Stable across releases (a single tag's manifest URL would
    /// 404 for prereleases via `/releases/latest/`, which is why
    /// we don't use that anymore).
    ///
    /// Path uses the `tray-` prefix per the release-track split
    /// (the release design notes).
    /// `release-manifests/{tray-alpha,tray-beta,tray-rc,tray-live}.json`
    /// hold Tauri updater manifests; the platform (server + web
    /// container images) has no updater manifest because it doesn't
    /// auto-update on the client side.
    pub fn manifest_url(&self) -> String {
        format!(
            "https://raw.githubusercontent.com/TheCodeSaiyan/StarStats-Platform/main/release-manifests/tray-{}.json",
            self.as_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_none_regression_flags_cleared_auth_fields() {
        let prev = Config {
            remote_sync: RemoteSyncConfig {
                claimed_handle: Some("Daisy".into()),
                access_token: Some("tok".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        // Both auth fields wiped — the exact synthetic-config-wipe shape.
        let next = Config {
            remote_sync: RemoteSyncConfig {
                claimed_handle: None,
                access_token: None,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            silent_none_regressions(&prev, &next),
            vec!["remote_sync.claimed_handle", "remote_sync.access_token"]
        );
    }

    #[test]
    fn write_atomic_overwrites_existing_and_leaves_no_temp() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "starstats-cfg-atomic-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        // A shorter payload must fully REPLACE longer pre-existing content,
        // never leave a truncated hybrid (the crash-mid-write failure mode).
        std::fs::write(&path, b"old-and-much-longer-content").unwrap();
        write_atomic(&path, b"new").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        // A successful write leaves no sibling temp file behind.
        assert!(!path.with_extension("toml.tmp").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn secrets_are_never_serialised_to_toml() {
        // M-T6: the device JWT and org bearer are `#[serde(skip)]`, so neither
        // may appear in the on-disk config even when set in memory (they live
        // in the keychain).
        let cfg = Config {
            remote_sync: RemoteSyncConfig {
                access_token: Some("secret-jwt".into()),
                claimed_handle: Some("Daisy".into()),
                ..Default::default()
            },
            org_connector: OrgConnectorConfig {
                enabled: true,
                platform_url: Some("https://orgs.example".into()),
                bearer_token: Some("secret-bearer".into()),
            },
            ..Default::default()
        };
        let toml = toml::to_string_pretty(&cfg).expect("serialise");
        assert!(!toml.contains("secret-jwt"), "device token leaked: {toml}");
        assert!(!toml.contains("access_token"));
        assert!(!toml.contains("secret-bearer"), "org bearer leaked: {toml}");
        assert!(!toml.contains("bearer_token"));
        // Non-secret fields still round-trip so we know serialisation works.
        assert!(toml.contains("Daisy"));
        assert!(toml.contains("orgs.example"));
    }

    #[test]
    fn silent_none_regression_ignores_unchanged_and_pairing() {
        // Unchanged populated fields → no regression.
        let populated = Config {
            remote_sync: RemoteSyncConfig {
                claimed_handle: Some("Daisy".into()),
                access_token: Some("tok".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(silent_none_regressions(&populated, &populated).is_empty());

        // None → Some (a fresh pairing) is the opposite of a wipe.
        let empty = Config::default();
        let paired = Config {
            remote_sync: RemoteSyncConfig {
                claimed_handle: Some("Daisy".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(silent_none_regressions(&empty, &paired).is_empty());
    }

    #[test]
    fn from_version_maps_known_prerelease_suffixes() {
        assert_eq!(
            ReleaseChannel::from_version("0.0.1-alpha"),
            ReleaseChannel::Alpha
        );
        assert_eq!(
            ReleaseChannel::from_version("0.3.12-alpha.1"),
            ReleaseChannel::Alpha
        );
        assert_eq!(
            ReleaseChannel::from_version("0.0.1-beta"),
            ReleaseChannel::Beta
        );
        assert_eq!(
            ReleaseChannel::from_version("1.0.0-beta.2"),
            ReleaseChannel::Beta
        );
        assert_eq!(ReleaseChannel::from_version("1.0.0-rc"), ReleaseChannel::Rc);
        assert_eq!(
            ReleaseChannel::from_version("1.0.0-rc.4"),
            ReleaseChannel::Rc
        );
    }

    #[test]
    fn from_version_treats_bare_version_as_live() {
        assert_eq!(ReleaseChannel::from_version("1.0.0"), ReleaseChannel::Live);
        assert_eq!(ReleaseChannel::from_version("0.0.1"), ReleaseChannel::Live);
    }

    #[test]
    fn from_version_falls_back_to_live_for_unknown_suffix() {
        // Unknown prerelease tokens are conservative: don't silently
        // accept random text as a real channel.
        assert_eq!(
            ReleaseChannel::from_version("1.0.0-canary"),
            ReleaseChannel::Live
        );
        assert_eq!(ReleaseChannel::from_version("1.0.0-"), ReleaseChannel::Live);
    }

    #[test]
    fn default_channel_is_live_regardless_of_build_version() {
        // The default is intentionally fixed to Live so users on a
        // pre-release binary still get a "stable" Updates surface by
        // default. The build's own channel (parsed via
        // `from_version`) is only relevant when the user explicitly
        // opts into matching it.
        assert_eq!(ReleaseChannel::default(), ReleaseChannel::Live);
    }

    #[test]
    fn missing_dismissed_health_defaults_empty() {
        // Simulate a TOML written before the field existed.
        let toml_str = r#"
            gamelog_path = "/tmp/Game.log"
            auto_update_check = false
            release_channel = "alpha"
            debug_logging = false
            theme = "stanton"

            [remote_sync]
            enabled = false
            api_url = "https://api.example"
            claimed_handle = "test"
            access_token = "tok"
            interval_secs = 60
            batch_size = 200
        "#;
        let cfg: Config = toml::from_str(toml_str).expect("parse legacy config");
        assert!(cfg.dismissed_health.is_empty());
    }

    #[test]
    fn dismissed_health_round_trips() {
        let mut cfg = Config::default();
        cfg.dismissed_health.push(crate::health::DismissedHealth {
            id: crate::health::HealthId::UpdateAvailable,
            fingerprint:
                "[\"update_available\",{\"id\":\"update_available\",\"version\":\"0.4.1\"}]".into(),
            dismissed_at: chrono::Utc::now(),
        });
        let s = toml::to_string_pretty(&cfg).expect("serialise");
        let round: Config = toml::from_str(&s).expect("deserialise");
        assert_eq!(round.dismissed_health.len(), 1);
        assert_eq!(
            round.dismissed_health[0].id,
            crate::health::HealthId::UpdateAvailable
        );
    }

    #[test]
    fn as_str_round_trips_through_serde() {
        for c in [
            ReleaseChannel::Alpha,
            ReleaseChannel::Beta,
            ReleaseChannel::Rc,
            ReleaseChannel::Live,
        ] {
            let json = serde_json::to_string(&c).unwrap();
            // serde renders enum variants quoted; strip quotes to compare.
            assert_eq!(json.trim_matches('"'), c.as_str());
        }
    }

    #[test]
    fn channel_mismatch_ack_round_trips_through_serde() {
        let cfg = Config {
            channel_mismatch_ack: Some("beta".to_string()),
            ..Config::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.channel_mismatch_ack, Some("beta".to_string()));
    }

    #[test]
    fn channel_mismatch_ack_defaults_to_none() {
        assert_eq!(Config::default().channel_mismatch_ack, None);
    }

    #[test]
    fn channel_mismatch_ack_is_optional_in_toml() {
        // Older configs on disk (written before this field existed) must
        // round-trip cleanly without the field present.
        let toml_text = r#"
            gamelog_path = "/tmp/Game.log"
            auto_update_check = false
            release_channel = "live"
            debug_logging = false
            theme = "stanton"
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(cfg.channel_mismatch_ack, None);
    }

    #[test]
    fn theme_default_is_stanton() {
        assert_eq!(Theme::default(), Theme::Stanton);
        assert_eq!(Config::default().theme, Theme::Stanton);
    }

    #[test]
    fn theme_round_trips_through_serde() {
        for t in [Theme::Stanton, Theme::Pyro, Theme::Terra, Theme::Nyx] {
            let json = serde_json::to_string(&t).unwrap();
            assert_eq!(json.trim_matches('"'), t.as_str());
            let parsed: Theme = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, t);
        }
    }

    #[test]
    fn config_without_theme_field_deserialises_to_stanton() {
        // Backward-compat: configs persisted before the theme field
        // existed must still load. `#[serde(default)]` on Config
        // covers absent fields by inserting Theme::default().
        let toml_text = "auto_update_check = true\n";
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(cfg.theme, Theme::Stanton);
    }

    #[test]
    fn theme_wave_speed_default_is_normal() {
        assert_eq!(Config::default().theme_wave_speed, "normal");
    }

    #[test]
    fn theme_wave_speed_round_trips_through_serde() {
        let cfg = Config {
            theme_wave_speed: "fast".to_string(),
            ..Config::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.theme_wave_speed, "fast");
    }

    #[test]
    fn config_without_theme_wave_speed_field_deserialises_to_normal() {
        // Backward-compat: configs persisted before this field existed
        // (including the fixture used by `missing_dismissed_health_defaults_empty`
        // above) must still load, defaulting to "normal" rather than
        // the bare `String::default()` (empty string).
        let toml_str = r#"
            gamelog_path = "/tmp/Game.log"
            auto_update_check = false
            release_channel = "alpha"
            debug_logging = false
            theme = "stanton"

            [remote_sync]
            enabled = false
            api_url = "https://api.example"
            claimed_handle = "test"
            access_token = "tok"
            interval_secs = 60
            batch_size = 200
        "#;
        let cfg: Config = toml::from_str(toml_str).expect("parse legacy config");
        assert_eq!(cfg.theme_wave_speed, "normal");
    }

    #[test]
    fn default_remote_sync_api_url_is_public_origin() {
        let cfg = Config::default();
        assert_eq!(
            cfg.remote_sync.api_url.as_deref(),
            Some(DEFAULT_API_URL),
            "fresh installs should default to the public StarStats API",
        );
    }

    #[test]
    fn config_without_api_url_field_deserialises_to_default() {
        // Backward-compat: configs persisted before the default
        // landed (or without the field set) should inherit the new
        // default via #[serde(default)] on RemoteSyncConfig.
        let toml_text = "[remote_sync]\nenabled = true\n";
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(cfg.remote_sync.api_url.as_deref(), Some(DEFAULT_API_URL));
    }

    #[test]
    fn config_without_priority_fields_inherits_defaults() {
        // Backward-compat: any config.toml written before the
        // priority-lanes feature landed must still parse, with the
        // new fields auto-filled from `default_priority_*` helpers.
        let toml_text = "[remote_sync]\nenabled = true\ninterval_secs = 120\n";
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(
            cfg.remote_sync.interval_secs, 120,
            "explicit field round-trips"
        );
        assert_eq!(
            cfg.remote_sync.priority_interval_secs, 5,
            "missing field uses the fast-lane default"
        );
        let expected: Vec<String> = DEFAULT_URGENT_TYPES.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            cfg.remote_sync.priority_event_types, expected,
            "missing field uses the canonical urgent-event set"
        );
    }

    #[test]
    fn config_with_explicit_priority_fields_round_trips() {
        let toml_text = r#"
[remote_sync]
enabled = true
priority_interval_secs = 2
priority_event_types = ["location_changed", "player_death"]
"#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(cfg.remote_sync.priority_interval_secs, 2);
        assert_eq!(
            cfg.remote_sync.priority_event_types,
            vec!["location_changed".to_string(), "player_death".to_string()]
        );
    }

    #[test]
    fn empty_priority_event_types_disables_fast_lane_semantically() {
        // The empty-list contract is: no fast lane, everything bulk.
        // We exercise the serialisation round-trip here; the actual
        // worker-side semantics are covered by sync.rs tests.
        let toml_text = "[remote_sync]\npriority_event_types = []\n";
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert!(cfg.remote_sync.priority_event_types.is_empty());
    }

    // -- effective_web_origin / derive_web_origin_from_api_url -------
    //
    // Regression coverage for the "Open on web takes you to the API
    // subdomain" bug. The TS code used to fall back to api_url raw —
    // the fix moved the rewrite Rust-side so there's one resolution
    // path and one place to test it.

    #[test]
    fn effective_web_origin_prefers_explicit_value() {
        let cfg = Config {
            web_origin: Some("https://custom.example".to_string()),
            remote_sync: RemoteSyncConfig {
                api_url: Some("https://api.starstats.app".to_string()),
                ..RemoteSyncConfig::default()
            },
            ..Config::default()
        };
        assert_eq!(
            cfg.effective_web_origin().as_deref(),
            Some("https://custom.example")
        );
    }

    #[test]
    fn effective_web_origin_strips_trailing_slashes_from_explicit_value() {
        let cfg = Config {
            web_origin: Some("https://custom.example///".to_string()),
            ..Config::default()
        };
        assert_eq!(
            cfg.effective_web_origin().as_deref(),
            Some("https://custom.example")
        );
    }

    #[test]
    fn effective_web_origin_treats_blank_explicit_value_as_unset() {
        let cfg = Config {
            web_origin: Some("   ".to_string()),
            remote_sync: RemoteSyncConfig {
                api_url: Some("https://api.starstats.app".to_string()),
                ..RemoteSyncConfig::default()
            },
            ..Config::default()
        };
        assert_eq!(
            cfg.effective_web_origin().as_deref(),
            Some("https://starstats.app")
        );
    }

    #[test]
    fn effective_web_origin_derives_from_api_url_when_unset() {
        let cfg = Config::default(); // ships DEFAULT_API_URL
        assert_eq!(
            cfg.effective_web_origin().as_deref(),
            Some("https://starstats.app")
        );
    }

    #[test]
    fn effective_web_origin_returns_none_when_both_unset() {
        let cfg = Config {
            remote_sync: RemoteSyncConfig {
                api_url: None,
                ..RemoteSyncConfig::default()
            },
            ..Config::default()
        };
        assert!(cfg.effective_web_origin().is_none());
    }

    #[test]
    fn derive_web_origin_strips_api_prefix_from_hostname() {
        assert_eq!(
            derive_web_origin_from_api_url("https://api.starstats.app"),
            Some("https://starstats.app".to_string())
        );
        assert_eq!(
            derive_web_origin_from_api_url("https://api.starstats.app/"),
            Some("https://starstats.app".to_string())
        );
    }

    #[test]
    fn derive_web_origin_preserves_scheme_and_port() {
        assert_eq!(
            derive_web_origin_from_api_url("http://api.example.test:8080/v1"),
            Some("http://example.test:8080".to_string())
        );
    }

    #[test]
    fn derive_web_origin_discards_path() {
        // Origin = scheme + authority; the deep-link path comes from
        // the caller, never from the api_url.
        assert_eq!(
            derive_web_origin_from_api_url("https://api.starstats.app/v1/healthz"),
            Some("https://starstats.app".to_string())
        );
    }

    #[test]
    fn derive_web_origin_ignores_api_in_path_segment() {
        // The `api.` prefix only counts on the HOST, not anywhere in
        // the URL. A user pointing api_url at a path under a non-api
        // host should not silently rewrite.
        assert_eq!(
            derive_web_origin_from_api_url("https://example.com/api.bar"),
            None
        );
    }

    #[test]
    fn derive_web_origin_returns_none_for_hosts_without_api_prefix() {
        // Local dev users on `localhost`, raw IPs, or custom
        // hostnames get None — the rewrite is best-effort, not magical.
        // The "Open on web" affordance renders disabled in that case.
        assert_eq!(
            derive_web_origin_from_api_url("http://localhost:8080"),
            None
        );
        assert_eq!(
            derive_web_origin_from_api_url("http://127.0.0.1:3000"),
            None
        );
        assert_eq!(derive_web_origin_from_api_url("https://example.com"), None);
    }

    #[test]
    fn derive_web_origin_rejects_malformed_inputs() {
        assert_eq!(derive_web_origin_from_api_url(""), None);
        assert_eq!(derive_web_origin_from_api_url("   "), None);
        assert_eq!(derive_web_origin_from_api_url("not-a-url"), None);
        assert_eq!(derive_web_origin_from_api_url("://no-scheme.example"), None);
        // host == "api." with nothing after → would yield empty
        // authority; the helper returns None rather than handing back
        // `https://`.
        assert_eq!(derive_web_origin_from_api_url("https://api."), None);
    }

    #[test]
    fn parser_enable_v2_metadata_absent_field_defaults_on() {
        // A config missing the field — the existing paired base and
        // fresh installs — opts INTO capture. `default = "default_true"`
        // makes the absent value `true`, so the parser-upgrade pipeline
        // (local capture + inference) is what ships unless a user
        // explicitly opts out. Capture is local-only; submission stays
        // an explicit, redaction-reviewed action.
        let toml_text = r#"
            [remote_sync]
            enabled = false
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert!(cfg.parser_enable_v2_metadata);
        assert!(cfg.v2_metadata_enabled());
    }

    #[test]
    fn parser_enable_v2_metadata_explicit_false_is_honored() {
        // The opt-out path: a user who sets the flag false in
        // config.toml keeps capture OFF even though the default is now
        // on. The explicit value always wins over the serde default.
        let toml_text = r#"
            parser_enable_v2_metadata = false
            [remote_sync]
            enabled = false
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert!(!cfg.parser_enable_v2_metadata);
        assert!(!cfg.v2_metadata_enabled());
    }

    #[test]
    fn parser_enable_v2_metadata_explicit_true_round_trips() {
        let toml_text = r#"
            parser_enable_v2_metadata = true
            [remote_sync]
            enabled = false
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert!(cfg.parser_enable_v2_metadata);
        assert!(cfg.v2_metadata_enabled());
        // Round-trip through TOML to confirm it serialises back.
        let s = toml::to_string_pretty(&cfg).unwrap();
        let round: Config = toml::from_str(&s).unwrap();
        assert!(round.parser_enable_v2_metadata);
    }

    #[test]
    fn parser_enable_v2_metadata_default_struct_is_on() {
        // Config::default() opts into capture, matching the serde
        // default so a programmatically-constructed config and a
        // deserialized-with-missing-field config agree.
        let cfg = Config::default();
        assert!(cfg.parser_enable_v2_metadata);
        assert!(cfg.v2_metadata_enabled());
    }

    #[test]
    fn autostart_enabled_absent_field_defaults_to_none() {
        // Configs persisted before the field existed must load with
        // `autostart_enabled = None`. The setup-closure treats None as
        // "first run, opt in" so legacy users land in the same default
        // state as a fresh install.
        let toml_text = r#"
            [remote_sync]
            enabled = false
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert!(cfg.autostart_enabled.is_none());
    }

    #[test]
    fn autostart_enabled_true_round_trips() {
        let cfg = Config {
            autostart_enabled: Some(true),
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).expect("serialise");
        let round: Config = toml::from_str(&s).expect("deserialise");
        assert_eq!(round.autostart_enabled, Some(true));
    }

    #[test]
    fn autostart_enabled_false_round_trips() {
        let cfg = Config {
            autostart_enabled: Some(false),
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).expect("serialise");
        let round: Config = toml::from_str(&s).expect("deserialise");
        assert_eq!(round.autostart_enabled, Some(false));
    }

    #[test]
    fn autostart_enabled_default_struct_is_none() {
        // Default::default() must keep the field unset so the
        // first-run-opt-in path in main.rs::setup runs exactly once
        // per install.
        let cfg = Config::default();
        assert!(cfg.autostart_enabled.is_none());
    }

    #[test]
    fn config_preserves_custom_api_url() {
        // A user pointing at a custom / dev instance keeps their
        // URL; the default only applies when the field is absent.
        let toml_text = r#"
            [remote_sync]
            enabled = true
            api_url = "http://localhost:8080"
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(
            cfg.remote_sync.api_url.as_deref(),
            Some("http://localhost:8080")
        );
    }

    #[test]
    fn sync_with_cloud_defaults_to_false() {
        let cfg = Config::default();
        assert!(!cfg.sync_with_cloud);
    }

    #[test]
    fn legacy_config_without_sync_with_cloud_field_deserialises_to_false() {
        // An on-disk TOML from a pre-cloud-sync release.
        let toml = r#"
            gamelog_path = "/tmp/Game.log"
            debug_logging = false
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(!cfg.sync_with_cloud);
    }

    #[test]
    fn sync_with_cloud_round_trips_through_toml() {
        let cfg = Config {
            sync_with_cloud: true,
            ..Config::default()
        };
        let s = toml::to_string(&cfg).unwrap();
        let parsed: Config = toml::from_str(&s).unwrap();
        assert!(parsed.sync_with_cloud);
    }
}

fn default_auto_update_check() -> bool {
    true
}

/// Public production StarStats API origin. Used as the default
/// `RemoteSyncConfig.api_url` so a fresh install can hit Enable and
/// proceed straight to pairing without first hunting down a URL.
/// Users on custom instances override via Settings.
pub const DEFAULT_API_URL: &str = "https://api.starstats.app";

/// Event types that ride the "fast" lane — drained on the
/// `priority_interval_secs` schedule (default 5s) rather than the
/// bulk `interval_secs` schedule (default 60s). Chosen for
/// player-visible "did that just happen?" signal: where I am, what
/// just killed me/my ship, and session boundaries. Stays a `&'static
/// [&'static str]` so it's cheap to default-clone into `Vec<String>`
/// per user config without runtime allocation churn.
///
/// Why these names: they're the literal `event_type` strings the
/// classifier emits (see `starstats_core::metadata::event_type_key`).
/// Mismatched names silently fail-soft — the filter just wouldn't
/// match — so this list is authoritative.
pub const DEFAULT_URGENT_TYPES: &[&str] = &[
    "location_changed",
    "player_death",
    "actor_death",
    "vehicle_destruction",
    "quantum_target_selected",
    "session_end",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSyncConfig {
    pub enabled: bool,
    /// Base URL of the StarStats API. Defaults to the public
    /// production origin (`DEFAULT_API_URL`). Override to point at
    /// a custom server or a local dev instance.
    pub api_url: Option<String>,
    /// RSI handle the user claims. Server cross-checks this against
    /// the bearer token's `preferred_username`; mismatch → 403.
    pub claimed_handle: Option<String>,
    /// Device JWT issued by the StarStats API (bearer credential).
    /// M-T6: never serialised — it lives in the OS keychain
    /// (`secret::ACCOUNT_DEVICE_TOKEN`), hydrated onto this field by
    /// `config::load` and written at pairing (`redeem_pair`). `#[serde(skip)]`
    /// keeps it out of both `config.toml` and the Tauri IPC emit to React.
    #[serde(skip)]
    pub access_token: Option<String>,
    /// How often the BULK lane drains. Default 60 s. Bulk handles
    /// everything that isn't in `priority_event_types`.
    #[serde(default = "default_sync_interval_secs")]
    pub interval_secs: u64,
    /// Max events per batch. Above this we split — server caps batch
    /// size and we get clean partial-success accounting.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// How often the PRIORITY lane drains. Default 5 s. The priority
    /// lane only handles event types listed in `priority_event_types`
    /// — leave the default list to get a snappy "where am I /
    /// what just happened" experience without paying for fast bulk
    /// pulls.
    #[serde(default = "default_priority_interval_secs")]
    pub priority_interval_secs: u64,
    /// Event-type strings that ride the fast lane. Empty list
    /// disables the priority lane entirely — everything drains on
    /// the bulk schedule. Names must match the classifier's
    /// `event_type` keys (see `DEFAULT_URGENT_TYPES`).
    #[serde(default = "default_priority_event_types")]
    pub priority_event_types: Vec<String>,
    /// Master switch for burst catch-up drains. When true (default) a
    /// lane that just shipped a FULL page keeps draining back-to-back
    /// instead of sleeping `interval_secs`, until the queue is empty.
    /// Turning it off restores the strict one-batch-per-interval
    /// cadence.
    #[serde(default = "default_true")]
    pub catch_up_enabled: bool,
    /// Events per batch while a lane is catching up on a backlog.
    /// Deliberately much larger than `batch_size`: the steady-state
    /// value is tuned for latency (ship the handful of events that
    /// just happened), the catch-up value for throughput (drain a
    /// six-figure queue in minutes, not days). Only ever used when
    /// the previous page came back full, and only when Star Citizen
    /// is NOT running — in-game catch-up stays on `batch_size` so the
    /// uplink never competes with the session.
    #[serde(default = "default_catch_up_batch_size")]
    pub catch_up_batch_size: usize,
    /// Ceiling on the ESTIMATED JSON size of a single `/v1/ingest`
    /// body, in bytes. A page read from SQLite is split into
    /// byte-bounded chunks before any send, so a large
    /// `catch_up_batch_size` can never produce a body the server
    /// rejects with 413. Kept comfortably under the server's
    /// `DefaultBodyLimit` for `/v1/ingest`.
    #[serde(default = "default_max_batch_bytes")]
    pub max_batch_bytes: usize,
}

impl Default for RemoteSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_url: Some(DEFAULT_API_URL.to_string()),
            claimed_handle: None,
            access_token: None,
            interval_secs: default_sync_interval_secs(),
            batch_size: default_batch_size(),
            priority_interval_secs: default_priority_interval_secs(),
            priority_event_types: default_priority_event_types(),
            catch_up_enabled: true,
            catch_up_batch_size: default_catch_up_batch_size(),
            max_batch_bytes: default_max_batch_bytes(),
        }
    }
}

fn default_sync_interval_secs() -> u64 {
    60
}

fn default_batch_size() -> usize {
    200
}

/// Catch-up page size. 2000 envelopes at the observed ~600 B/envelope
/// is roughly 1.2 MB — well inside [`default_max_batch_bytes`], and
/// 10x the steady-state page, so a 300k-event backlog is ~150 requests
/// rather than ~1500.
fn default_catch_up_batch_size() -> usize {
    2000
}

/// 3 MB. The server's `/v1/ingest` body limit is larger; the gap is
/// deliberate headroom for the JSON scaffolding the byte estimator
/// under-counts.
fn default_max_batch_bytes() -> usize {
    3 * 1024 * 1024
}

fn default_priority_interval_secs() -> u64 {
    5
}

fn default_priority_event_types() -> Vec<String> {
    DEFAULT_URGENT_TYPES.iter().map(|s| s.to_string()).collect()
}

fn project_dirs() -> Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("app", "StarStats", "tray")
        .context("could not resolve user config/data directories")
}

pub fn config_dir() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    let dir = dirs.config_dir().to_path_buf();
    std::fs::create_dir_all(&dir).context("create config dir")?;
    Ok(dir)
}

pub fn data_dir() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    let dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir).context("create data dir")?;
    Ok(dir)
}

pub fn load() -> Result<Config> {
    let path = config_dir()?.join("config.toml");
    let mut cfg: Config = if !path.exists() {
        Config::default()
    } else {
        let text = std::fs::read_to_string(&path).context("read config.toml")?;
        toml::from_str(&text).context("parse config.toml")?
    };
    // M-T6: the device JWT and org bearer are `#[serde(skip)]`, so they're
    // never in the file — hydrate them onto the struct from the OS keychain
    // (the source of truth).
    hydrate_secrets(&mut cfg);
    Ok(cfg)
}

pub fn save(cfg: &Config) -> Result<()> {
    // Serialize every writer process-wide. There are ≥6 concurrent
    // read-modify-write callers (pairing, sync-preset changes, token
    // refresh, unpair, …); without this lock their `fs::write`s could
    // interleave a half-written file, and two racing save()s could clobber
    // each other's bytes. Poison is recovered rather than propagated — one
    // panicked writer must not brick every future save (the poisoned-mutex
    // bricking class we avoid elsewhere).
    static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SAVE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let path = config_dir()?.join("config.toml");

    // Diagnostic for the silent claimed_handle/access_token Some→None
    // regression class (the synthetic-config-laundering bug that wiped
    // `claimed_handle` and left the tray unable to spawn sync workers).
    // Compare the incoming config against the current on-disk state; a
    // previously-populated remote-sync auth field going None is almost
    // always a bug, so log a backtrace pointing at the caller. The save
    // still proceeds — a deliberate unpair legitimately clears them, and
    // a read/parse hiccup here must never block persistence.
    if let Ok(prev) = load() {
        for field in silent_none_regressions(&prev, cfg) {
            tracing::warn!(
                field,
                backtrace = %std::backtrace::Backtrace::force_capture(),
                "config: a populated remote-sync auth field is being cleared to None on save; \
                 sync workers will not spawn until the device is re-paired. If this was not a \
                 deliberate unpair, it is the synthetic-config wipe — capture this backtrace."
            );
        }
    }

    // The device JWT is `#[serde(skip)]`, so `to_string_pretty` already omits
    // it — the file never contains the token (M-T6). The keychain is written
    // at the explicit pairing/unpair sites, not here.
    let text = toml::to_string_pretty(cfg).context("serialise config")?;
    write_atomic(&path, text.as_bytes())?;
    Ok(())
}

/// Overlay the keychain-stored secrets (device JWT + org bearer) onto `cfg` —
/// the source of truth since M-T6. Called by `load` after parsing, and by
/// `save_config` after it receives a secret-less `Config` over IPC from React,
/// so downstream readers (`sync::start`, cloud-sync `device_id` extraction,
/// the org connector) see the real values. Keychain errors are logged and
/// non-fatal — a broken keychain surfaces as "not configured", never a crash.
pub(crate) fn hydrate_secrets(cfg: &mut Config) {
    cfg.remote_sync.access_token = read_secret(crate::secret::ACCOUNT_DEVICE_TOKEN);
    cfg.org_connector.bearer_token = read_secret(crate::secret::ACCOUNT_ORG_BEARER);
}

/// Read one keychain secret by account, mapping any keychain error to `None`
/// (logged) so a broken keychain degrades to "not configured" rather than
/// propagating a startup failure.
fn read_secret(account: &str) -> Option<String> {
    match crate::secret::SecretStore::new(account).and_then(|store| store.get()) {
        Ok(value) => value,
        Err(e) => {
            tracing::warn!(error = %e, account, "read secret from keychain failed");
            None
        }
    }
}

/// Write `bytes` to `path` atomically: write a sibling temp file, flush it to
/// disk, then rename it over the target. Rename within a directory replaces
/// the destination in a single step on both POSIX (`rename(2)`) and Windows
/// (`MoveFileExW` with `REPLACE_EXISTING`, which `std::fs::rename` uses), so a
/// crash mid-write leaves either the old complete file or the new complete
/// file — never the truncated hybrid that a bare `fs::write` produces. This is
/// the durability half of the config-wipe-on-crash fix; the caller holds the
/// process-wide `SAVE_LOCK` so the temp file can't race another writer.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    // Sibling temp in the SAME directory → same filesystem → the rename is a
    // true atomic replace rather than a cross-device copy.
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("create temp config {}", tmp.display()))?;
        f.write_all(bytes).context("write temp config")?;
        f.sync_all().context("fsync temp config")?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("atomic rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Names of remote-sync auth fields that went `Some` → `None` between
/// the on-disk `prev` config and the `next` one about to be written.
/// These fields (`claimed_handle`, `access_token`) gate the sync
/// workers, so a silent clear is the signature of the config-wipe bug.
/// Pure — the caller decides how to surface the result.
fn silent_none_regressions(prev: &Config, next: &Config) -> Vec<&'static str> {
    let mut regressed = Vec::new();
    if prev.remote_sync.claimed_handle.is_some() && next.remote_sync.claimed_handle.is_none() {
        regressed.push("remote_sync.claimed_handle");
    }
    if prev.remote_sync.access_token.is_some() && next.remote_sync.access_token.is_none() {
        regressed.push("remote_sync.access_token");
    }
    regressed
}
