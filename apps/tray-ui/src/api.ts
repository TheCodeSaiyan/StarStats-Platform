import { invoke } from '@tauri-apps/api/core';

/**
 * What kind of artifact a `DiscoveredLog` points at. Matches the Rust
 * `LogKind` enum (snake_case-serialised). Today only `channel_live`
 * is actually tailed by the client; the rest are surfaced for visibility
 * and as a seed for future ingest paths.
 */
export type LogKind =
  | 'channel_live'
  | 'channel_archived'
  | 'crash_report'
  | 'launcher_log';

export interface DiscoveredLog {
  channel: string;
  kind: LogKind;
  path: string;
  size_bytes: number;
}

export interface TailStats {
  current_path: string | null;
  bytes_read: number;
  lines_processed: number;
  events_recognised: number;
  last_event_at: string | null;
  last_event_type: string | null;
  lines_structural_only: number;
  lines_skipped: number;
  lines_noise: number;
}

export interface EventCount {
  event_type: string;
  count: number;
}

export interface SyncStats {
  last_attempt_at: string | null;
  last_success_at: string | null;
  last_error: string | null;
  batches_sent: number;
  events_accepted: number;
  events_duplicate: number;
  events_rejected: number;
}

export interface AccountStatus {
  /// True once the API has rejected the device token (401/403).
  /// Cleared by a successful re-pair.
  auth_lost: boolean;
  /// Mirror of `MeResponse.email_verified`. `null` until the first
  /// successful `GET /v1/auth/me` call lands.
  email_verified: boolean | null;
}

export interface HangarStats {
  last_attempt_at: string | null;
  last_success_at: string | null;
  last_error: string | null;
  ships_pushed: number;
  last_skip_reason: string | null;
}

export interface RsiCookieStatus {
  configured: boolean;
  preview: string | null;
}

/**
 * Status probe for the org-connector bearer token. The token itself
 * lives in the OS keychain (never in `Config`); this is the only shape
 * the renderer ever sees. `preview` is a redacted "…WXYZ" tail, never
 * the raw secret.
 */
export interface OrgBearerStatus {
  configured: boolean;
  preview: string | null;
}

export interface StatusResponse {
  tail: TailStats;
  sync: SyncStats;
  event_counts: EventCount[];
  total_events: number;
  discovered_logs: DiscoveredLog[];
  account: AccountStatus;
  hangar: HangarStats;
}

export interface RemoteSyncConfig {
  enabled: boolean;
  api_url: string | null;
  claimed_handle: string | null;
  access_token: string | null;
  /** How often the BULK lane drains (seconds). Defaults to 60. */
  interval_secs: number;
  batch_size: number;
  /** How often the PRIORITY lane drains (seconds). Defaults to 5. */
  priority_interval_secs: number;
  /**
   * Event-type strings that ride the fast lane. Empty list disables
   * the priority lane entirely. Defaults to the canonical urgent set
   * (location, deaths, vehicle destruction, quantum target, session
   * end) — see Rust-side `DEFAULT_URGENT_TYPES`.
   */
  priority_event_types: string[];
}

/**
 * Named sync-speed presets. Pairs of (priority_interval, bulk_interval)
 * are kept ~10x apart so the fast/bulk ratio stays sensible across
 * presets. `"custom"` is a UI-only marker: the Rust side returns the
 * current config unchanged when this is selected — actual numbers
 * change via `saveConfig` once the user edits a field.
 */
export type SyncPreset = 'fast' | 'balanced' | 'resource_saver' | 'custom';

export const SYNC_PRESETS: {
  id: SyncPreset;
  label: string;
  description: string;
  priorityInterval: number;
  bulkInterval: number;
}[] = [
  {
    id: 'fast',
    label: 'Fast',
    description: 'Priority every 3s, bulk every 30s — snappier "where am I" updates.',
    priorityInterval: 3,
    bulkInterval: 30,
  },
  {
    id: 'balanced',
    label: 'Balanced',
    description: 'Default. Priority every 5s, bulk every 60s.',
    priorityInterval: 5,
    bulkInterval: 60,
  },
  {
    id: 'resource_saver',
    label: 'Resource saver',
    description: 'Priority every 30s, bulk every 5min — lowest CPU/network.',
    priorityInterval: 30,
    bulkInterval: 300,
  },
  {
    id: 'custom',
    label: 'Custom',
    description: 'Pick your own intervals below.',
    priorityInterval: 0,
    bulkInterval: 0,
  },
];

/**
 * Match a current `RemoteSyncConfig` to its named preset, if any.
 * Returns `"custom"` when the intervals don't line up with one of
 * the canned pairs — the UI uses this to highlight the active radio.
 */
export function detectSyncPreset(cfg: RemoteSyncConfig): SyncPreset {
  for (const p of SYNC_PRESETS) {
    if (p.id === 'custom') continue;
    if (
      cfg.priority_interval_secs === p.priorityInterval &&
      cfg.interval_secs === p.bulkInterval
    ) {
      return p.id;
    }
  }
  return 'custom';
}

/**
 * Release channel the updater tracks. Each value maps to a manifest
 * file at `release-manifests/<channel>.json` on the StarStats main
 * branch. Client-side default is derived from the build's package
 * version (e.g. a `-beta` build defaults to `beta`); users can switch
 * via the Settings dropdown.
 */
export type ReleaseChannel = 'alpha' | 'beta' | 'rc' | 'live';

export const RELEASE_CHANNEL_LABELS: Record<ReleaseChannel, string> = {
  alpha: 'Alpha',
  beta: 'Beta',
  rc: 'Release Candidate',
  live: 'Live',
};

/**
 * Visual theme matching the four `[data-theme="..."]` blocks in
 * `starstats-tokens.css`. Serialised lowercase to match the Rust
 * `Theme` enum (snake_case serde).
 */
export type Theme = 'stanton' | 'pyro' | 'terra' | 'nyx';

/**
 * Settings → Appearance card metadata. Each entry maps a theme id to
 * a display label, a short tagline (lifted from the design tokens
 * comment header), and four palette swatches the picker renders.
 * Colour values are duplicated from `starstats-tokens.css` rather than
 * read at runtime so the swatch preview survives unknown future themes
 * without a CSS round-trip.
 */
export interface ThemeMeta {
  id: Theme;
  label: string;
  tagline: string;
  swatch: { bg: string; surface: string; accent: string; fg: string };
}

export const THEMES: ReadonlyArray<ThemeMeta> = [
  {
    id: 'stanton',
    label: 'Stanton',
    tagline: 'warm amber',
    swatch: { bg: '#0F0E12', surface: '#1A1820', accent: '#E8A23C', fg: '#ECE7DD' },
  },
  {
    id: 'pyro',
    label: 'Pyro',
    tagline: 'molten coral',
    swatch: { bg: '#100C0E', surface: '#1F1517', accent: '#F25C3F', fg: '#F2E6E0' },
  },
  {
    id: 'terra',
    label: 'Terra',
    tagline: 'cool teal',
    swatch: { bg: '#0B1014', surface: '#131C22', accent: '#4FB8A1', fg: '#E2EAEC' },
  },
  {
    id: 'nyx',
    label: 'Nyx',
    tagline: 'violet on cream',
    swatch: { bg: '#F4F1EC', surface: '#FFFFFF', accent: '#5B3FD9', fg: '#1B1722' },
  },
];

/**
 * Opt-in connector to a self-hosted org platform. All three fields must
 * be set for the connector to start. This block is intentionally excluded
 * from cloud-sync payloads — it is a per-install local link, not a
 * per-account preference.
 */
export interface OrgConnectorConfig {
  enabled: boolean;
  platform_url: string | null;
  bearer_token: string | null;
}

export interface Config {
  gamelog_path: string | null;
  remote_sync: RemoteSyncConfig;
  web_origin: string | null;
  /// Whether to automatically check for updates on app startup.
  /// Defaults to true server-side; the Updates card in Settings
  /// exposes a toggle.
  auto_update_check: boolean;
  /// Which release channel the in-app updater queries. Defaults to
  /// the channel this build was published on (parsed from the Cargo
  /// package version's prerelease suffix); users can switch via the
  /// Updates card. Changing this takes effect on the next "Check for
  /// updates" or app restart.
  release_channel: ReleaseChannel;
  /// Whether to write a daily-rolling `client.log` to the user
  /// data dir. Defaults to false so end users have no log clutter;
  /// toggle on from Settings → Updates to capture logs for a bug
  /// report. The panic-only log is always written.
  debug_logging: boolean;
  /// Opt-in: capture unrecognised `game.log` lines into the local
  /// review queue so you can submit privacy-safe "shapes" that help add
  /// new parser rules. Local-only until you explicitly submit; off by
  /// default. Takes effect on the next app restart.
  parser_enable_v2_metadata: boolean;
  /// Visual theme driving the `[data-theme="..."]` attribute on
  /// `<html>`. Defaults Stanton server-side; users switch via
  /// Settings → Appearance.
  theme: Theme;
  /// Theme-switch wave animation speed — one of `WAVE_SPEEDS`
  /// (`off`/`slow`/`normal`/`fast`). Drives the `[data-wave-speed]`
  /// attribute on `<html>` that `lib/theme-transition.ts` reads to
  /// resolve the sweep duration. Defaults `"normal"` server-side
  /// (`Config::default` in `config.rs`); the Appearance card exposes a
  /// Speed control alongside the theme swatches. Untyped `string`
  /// (not `WaveSpeed`) at this layer to mirror the Rust field and the
  /// core `UserPreferences.theme_wave_speed` wire shape — callers that
  /// render it validate with `isWaveSpeed` before trusting the value.
  theme_wave_speed: string;
  /// User preference for launching StarStats at system sign-in.
  /// `null` = first run (the tray applies the default-on policy and
  /// flips it to `true`); `true` / `false` = the user's explicit
  /// choice. Reads from the OS state, not this field, are exposed via
  /// `api.getAutostartEnabled()`.
  autostart_enabled: boolean | null;
  /// When true, this tray's theme and settings are stored on the
  /// paired account and synced to other devices. Off by default.
  /// Only meaningful when the tray is paired (remote_sync has a
  /// valid access_token + claimed_handle).
  sync_with_cloud: boolean;
  /** Opt-in: report the NAMES of unreadable log entry types (never the
   *  lines themselves) so a broken parser can be diagnosed. Default false. */
  share_unknown_tags: boolean;
  /**
   * Build-channel value the user previously dismissed the
   * channel-mismatch banner for. `null` means never dismissed.
   * Compared against the *running binary's* channel (from
   * `getBuildReleaseChannel`) to decide whether to re-surface
   * the banner after the user upgrades into a different channel's
   * build.
   */
  channel_mismatch_ack: string | null;
  /**
   * Opt-in connector to a self-hosted org platform. Intentionally
   * excluded from cloud-sync payloads — this is a per-install local
   * link, not a per-account preference.
   */
  org_connector: OrgConnectorConfig;
}


export interface PairOutcome {
  claimed_handle: string;
  label: string;
}

export interface UnknownSample {
  log_source: string;
  event_name: string;
  occurrences: number;
  first_seen: string;
  last_seen: string;
  sample_line: string;
  sample_body: string;
}

export interface ParseCoverageResponse {
  recognised: number;
  structural_only: number;
  skipped: number;
  noise: number;
  unknowns: UnknownSample[];
}

/** Snapshot of the launcher-log tailer. `current_path` is null when
 * no launcher logs were discovered locally. */
export interface LauncherStats {
  current_path: string | null;
  bytes_read: number;
  lines_processed: number;
  events_recognised: number;
  last_event_at: string | null;
  last_level: string | null;
  last_category: string | null;
  lines_skipped: number;
}

/** Snapshot of the crash-dir scanner. `last_crash_dir` is the most
 * recent crash on disk (newest-first by dir name). */
export interface CrashStats {
  last_scan_at: string | null;
  total_crashes_seen: number;
  last_crash_dir: string | null;
}

/** Snapshot of the rotated-log backfill task. `completed = true` means
 * the initial sweep finished; `false` means it's still scanning. */
export interface BackfillStats {
  completed: boolean;
  files_total: number;
  files_processed: number;
  files_already_done: number;
  lines_processed: number;
  events_recognised: number;
}

export interface SourceStats {
  launcher: LauncherStats;
  crashes: CrashStats;
  backfill: BackfillStats;
}

/**
 * Location resolved client-side (Rust) from the event's raw engine
 * string via the shared classifier. `slug` is present only when the
 * classifier is confident enough to link (catalog/fuzzy hit); otherwise
 * `display_name` is shown as plain text. Mirrors the Rust
 * `ResolvedLocation` in `commands.rs`.
 */
export interface ResolvedLocation {
  display_name: string;
  slug?: string;
  system?: string;
  tier: string;
}

export interface TimelineEntry {
  id: number;
  timestamp: string;
  event_type: string;
  summary: string;
  raw_line: string;
  /// Channel tag (LIVE/PTU/EPTU) the event was tailed from.
  log_source: string;
  synced: boolean;
  /// Resolved location, when the event carries one. Absent for
  /// placeless events.
  location?: ResolvedLocation;
}

export interface StorageStats {
  total_events: number;
  db_size_bytes: number;
}

/**
 * Result of the `search_events` Tauri command. The server filters by
 * `query` (substring match against event_type + payload, case-insensitive),
 * `type_filter` (exact event_type match), and paginates DESC by
 * (timestamp, id) using `before_id` as the cursor. `total` is the
 * total row count matching the current filter (independent of
 * pagination); `has_more` is true when more rows exist below the
 * returned page.
 */
export interface SearchEventsResult {
  entries: TimelineEntry[];
  total: number;
  has_more: boolean;
}

export interface ReparseStats {
  examined: number;
  updated: number;
  kept_unmatched: number;
  promoted_unknowns: number;
  /** Bursts retroactively detected over already-stored events. Each
   *  hit produces one `burst_summary` row; the original member rows
   *  are deleted. Sessions already collapsed at live-tail time are a
   *  no-op (idempotency key matches the live shape). */
  bursts_collapsed: number;
  /** Total per-line member rows deleted as part of `bursts_collapsed`.
   *  A single burst commonly absorbs 20+ rows. */
  members_suppressed: number;
  error: string | null;
}

/**
 * Result of `reingest_rotated_logs`. Distinct from `ReparseStats`:
 * Re-parse walks the local SQLite store to re-classify already-stored
 * events; Re-ingest walks the raw rotated `Game-*.log` files on disk
 * and feeds each line back through the classifier. The latter is the
 * only way to recover events that were `None`'d by an older parser
 * version (the body-line PlayerDeath events live only in the raw logs
 * because the v0.2.x parser couldn't recognise them).
 */
export interface ReingestStats {
  files_walked: number;
  files_failed: number;
  lines_processed: number;
  events_recognised: number;
  error: string | null;
}

export type TransactionKind = 'shop' | 'commodity_buy' | 'commodity_sell';
export type TransactionStatus =
  | 'pending'
  | 'confirmed'
  | 'rejected'
  | 'timed_out'
  | 'submitted';

export interface Transaction {
  kind: TransactionKind;
  status: TransactionStatus;
  started_at: string;
  confirmed_at: string | null;
  shop_id: string | null;
  item: string | null;
  quantity: number | null;
  raw_request: string;
  raw_response: string | null;
}

// === Health surface ===

export type Severity = 'error' | 'warn' | 'info';

export type HealthId =
  | 'gamelog_missing'
  | 'api_url_missing'
  | 'pair_missing'
  | 'auth_lost'
  | 'cookie_missing'
  | 'sync_failing'
  | 'hangar_skip'
  | 'email_unverified'
  | 'game_log_stale'
  | 'update_available'
  | 'disk_free_low';

export type SettingsField =
  | 'gamelog_path'
  | 'api_url'
  | 'pairing_code'
  | 'rsi_cookie'
  | 'updates';

export type HealthAction =
  | { kind: 'go_to_settings'; field: SettingsField }
  | { kind: 'retry_sync' }
  | { kind: 'refresh_hangar' }
  | { kind: 'open_url'; url: string };

export type HealthParams =
  | { id: 'gamelog_missing' }
  | { id: 'api_url_missing' }
  | { id: 'pair_missing' }
  | { id: 'auth_lost' }
  | { id: 'cookie_missing' }
  | { id: 'sync_failing'; last_error: string; attempts_since_success: number }
  | { id: 'hangar_skip'; reason: string; since: string }
  | { id: 'email_unverified' }
  | { id: 'game_log_stale'; last_event_at: string }
  | { id: 'update_available'; version: string }
  | { id: 'disk_free_low'; free_bytes: number };

export interface HealthItem {
  id: HealthId;
  severity: Severity;
  params: HealthParams;
  action: HealthAction | null;
  dismissible: boolean;
  fingerprint: string;
}

export interface ApiUrlCheck {
  ok: boolean;
  status: number | null;
  server_version: string | null;
  error: string | null;
}

// === Parser submissions ===

/**
 * Log channel an unknown-line capture came from. Mirrors the Rust
 * `LogSource` enum (`#[serde(rename_all = "lowercase")]`) so the strings
 * the Tauri bridge hands back deserialise cleanly. Tray captures from
 * the live channel today; the enum carries the other branches so a
 * future PTU/Eptu/etc. capture surfaces the correct channel server-side
 * for rule-scope decisions.
 */
export type LogSource =
  | 'live'
  | 'ptu'
  | 'eptu'
  | 'hotfix'
  | 'tech'
  | 'other';

export type PiiKind =
  | 'own_handle'
  | 'friend_handle'
  | 'shard_id'
  | 'geid'
  | 'ip_port';

export interface PiiToken {
  kind: PiiKind;
  start: number;
  end: number;
  suggested_redaction: string;
  default_redact: boolean;
}

/**
 * One unknown-line row out of the local SQLite cache. Mirrors the
 * Rust `UnknownLine` struct (snake_case serde). The review pane only
 * needs a subset of these fields; the rest are passed through to the
 * submission payload so the server-side reviewer has full context.
 */
export interface UnknownLine {
  id: string;
  raw_line: string;
  timestamp: string | null;
  shell_tag: string | null;
  partial_structured: Record<string, string>;
  context_before: string[];
  context_after: string[];
  game_build: string | null;
  channel: LogSource;
  interest_score: number;
  shape_hash: string;
  occurrence_count: number;
  first_seen: string;
  last_seen: string;
  detected_pii: PiiToken[];
  dismissed: boolean;
}

/**
 * One element of the `POST /v1/parser-submissions` batch. Mirrors the
 * Rust `ParserSubmission` struct. `client_anon_id` is a stable hash
 * the server uses to dedupe submissions from the same anonymous user
 * without identifying them — the bearer token does the auth.
 */
export interface ParserSubmission {
  shape_hash: string;
  raw_examples: string[];
  partial_structured?: Record<string, string>;
  shell_tag?: string;
  suggested_event_name?: string;
  suggested_field_names?: Record<string, string>;
  notes?: string;
  context_examples?: Array<{ before: string[]; after: string[] }>;
  game_build?: string;
  channel: LogSource;
  occurrence_count: number;
  client_anon_id: string;
  /**
   * The tray user's forced attribution choice for this submission.
   * `true` credits the paired account (server resolves the identity
   * from the device token); `false` posts anonymously (shown as
   * `@community`). The tray UI forces an explicit pick per submit —
   * there is no silent default here.
   */
  attributed: boolean;
}

export interface ParserSubmissionResponse {
  accepted: number;
  deduped: number;
  ids: string[];
}

export interface CookieCheck {
  ok: boolean;
  handle: string | null;
  error: string | null;
}

// === Roadmap "What's new" ===

/**
 * One roadmap item in the tray "What's new" queue. Mirrors the Rust
 * `WhatsNewItem` (snake_case serde). Canonical home for the type is
 * here on the api surface; `WhatsNewPane` re-exports it for callers.
 */
export interface WhatsNewItem {
  roadmap_item_id: string;
  slug: string;
  title: string;
  headline_status: string;
  latest_changelog_entry_id: string;
  latest_published_at: string;
  unread: boolean;
}

export interface WhatsNewResponse {
  items: WhatsNewItem[];
  seen_via_auth: boolean;
}

export const api = {
  getStatus: () => invoke<StatusResponse>('get_status'),
  getConfig: () => invoke<Config>('get_config'),
  saveConfig: (cfg: Config) => invoke<void>('save_config', { cfg }),
  /**
   * Apply a named sync-speed preset. Returns the resulting
   * `RemoteSyncConfig` so callers can refresh local state without a
   * follow-up `getConfig` round-trip. `"custom"` is a no-op on the
   * Rust side — handle it UI-side by revealing the raw number inputs.
   */
  setSyncPreset: (preset: SyncPreset) =>
    invoke<RemoteSyncConfig>('set_sync_preset', { preset }),
  getDiscoveredLogs: () => invoke<DiscoveredLog[]>('get_discovered_logs'),
  pairDevice: (apiUrl: string, code: string) =>
    invoke<PairOutcome>('pair_device', { api_url: apiUrl, code }),
  getParseCoverage: () =>
    invoke<ParseCoverageResponse>('get_parse_coverage'),
  getSessionSummaryText: () => invoke<string>('get_session_summary_text'),
  getSessionTimeline: (limit?: number) =>
    invoke<TimelineEntry[]>('get_session_timeline', { limit }),
  /**
   * Server-side search + pagination across the full event table.
   * `query` is a case-insensitive substring match over event_type +
   * payload JSON; `type_filter` is an exact event_type match;
   * `before_id` is the pagination cursor (omit for the first page).
   * The Rust side clamps `limit` to [1, 5000].
   */
  searchEvents: (params: {
    query?: string;
    type_filter?: string;
    before_id?: number | null;
    limit?: number;
  }) => invoke<SearchEventsResult>('search_events', params),
  listTransactions: (limit?: number, window_secs?: number) =>
    invoke<Transaction[]>('list_transactions', {
      limit,
      window_secs,
    }),
  getSourceStats: () => invoke<SourceStats>('get_source_stats'),
  getStorageStats: () => invoke<StorageStats>('get_storage_stats'),
  /** Cargo workspace version (e.g. "0.2.0-alpha") — matches the
   * GitHub release tag. Distinct from Tauri's getVersion() which
   * returns the numeric tauri.conf.json version (MSI-friendly). */
  getAppVersion: () => invoke<string>('get_app_version'),
  /**
   * Returns the running binary's compiled-in release channel
   * (parsed from the Cargo package version's prerelease suffix
   * at build time). Distinct from `Config.release_channel`,
   * which is the user-configured channel the updater queries.
   * A mismatch between the two drives the channel-mismatch
   * banner in the Updates card.
   */
  getBuildReleaseChannel: () => invoke<ReleaseChannel>('get_build_release_channel'),
  /** Re-run the current classifier over every stored event in place.
   * Idempotent on a stable rule set; safe to invoke from a button. */
  reparseEvents: () => invoke<ReparseStats>('reparse_events'),
  reingestRotatedLogs: () => invoke<ReingestStats>('reingest_rotated_logs'),
  refreshHangarNow: () => invoke<void>('refresh_hangar_now'),
  markEventAsNoise: (eventName: string) =>
    invoke<void>('mark_event_as_noise', { event_name: eventName }),
  refreshAccountInfo: () => invoke<AccountStatus>('refresh_account_info'),
  retrySyncNow: () => invoke<void>('retry_sync_now'),
  /**
   * Persistent count of rows the sync worker has quarantined (rows
   * whose `sent_at` starts with `__quarantined_`). Distinct from
   * `SyncStats.events_quarantined`, which is the lifetime-of-process
   * counter of how many times we've stamped a row; this is the
   * current DB-resident count and shrinks back to 0 after
   * `releaseQuarantined()` succeeds.
   */
  countQuarantined: () => invoke<number>('count_quarantined'),
  /**
   * Flip `sent_at` from `__quarantined_*` back to NULL on every
   * quarantined row, then kick the sync worker so the next drain
   * re-attempts them. Returns the count released. Recovery affordance
   * for mass-quarantine caused by transient batch-level 4xx that the
   * poison-pill bisection mis-attributed to each event.
   */
  releaseQuarantined: () => invoke<number>('release_quarantined'),
  setRsiCookie: (cookieValue: string) =>
    invoke<RsiCookieStatus>('set_rsi_cookie', { cookie_value: cookieValue }),
  clearRsiCookie: () => invoke<RsiCookieStatus>('clear_rsi_cookie'),
  getRsiCookieStatus: () => invoke<RsiCookieStatus>('get_rsi_cookie_status'),
  // Org-connector bearer token lives in the OS keychain, managed via
  // dedicated commands (not the config save path). IPC arg key is
  // byte-exact snake_case per the repo-wide invariant.
  setOrgBearer: (bearerToken: string) =>
    invoke<OrgBearerStatus>('set_org_bearer', { bearer_token: bearerToken }),
  clearOrgBearer: () => invoke<OrgBearerStatus>('clear_org_bearer'),
  getOrgBearerStatus: () =>
    invoke<OrgBearerStatus>('get_org_bearer_status'),
  getHealth: () => invoke<HealthItem[]>('get_health'),
  dismissHealth: (id: HealthId) => invoke<void>('dismiss_health', { id }),
  checkApiUrl: (url: string) => invoke<ApiUrlCheck>('check_api_url', { url }),
  checkRsiCookie: (cookie: string) => invoke<CookieCheck>('check_rsi_cookie', { cookie }),
  setUpdateAvailable: (version: string) => invoke<void>('set_update_available', { version }),
  listUnknownLines: () => invoke<UnknownLine[]>('list_unknown_lines'),
  countUnknownLines: () => invoke<number>('count_unknown_lines'),
  dismissUnknownLine: (shapeHash: string) =>
    invoke<void>('dismiss_unknown_line', { shape_hash: shapeHash }),
  submitUnknownLines: (payloads: ParserSubmission[]) =>
    invoke<ParserSubmissionResponse>('submit_unknown_lines', { payloads }),
  getAutostartEnabled: () => invoke<boolean>('get_autostart_enabled'),
  setAutostartEnabled: (enabled: boolean) =>
    invoke<void>('set_autostart_enabled', { enabled }),
  /**
   * Fetch one reference category via the Rust client.
   *
   * The WebView's CSP blocks cross-origin `fetch()` from the
   * frontend, so the catalogue listing has to come through the IPC
   * bridge. The Tauri command preserves the wire shape from
   * `/v1/reference/{category}` (entries with class_name +
   * display_name + slug + summary) — the typed parsing happens in
   * `apps/tray-ui/src/lib/reference.ts`.
   */
  getReferenceCategory: (apiUrl: string, category: string) =>
    invoke<{ entries: unknown[] }>('get_reference_category', {
      api_url: apiUrl,
      category,
    }),
  /**
   * Roadmap "What's new" queue. Rust-side fetch (CSP blocks the
   * cross-origin HTTP from the WebView) — mirrors `getReferenceCategory`.
   */
  getWhatsNew: () => invoke<WhatsNewResponse>('get_whats_new'),
  /**
   * Mark a roadmap item's latest changelog entry seen for the paired
   * account. Keys are byte-exact snake_case to match the Rust params
   * `item_id` / `entry_id` under `rename_all = "snake_case"`.
   */
  markWhatsNewSeen: (itemId: string, entryId: string) =>
    invoke<void>('mark_whats_new_seen', { item_id: itemId, entry_id: entryId }),
};
