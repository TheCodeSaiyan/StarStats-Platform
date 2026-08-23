import { useEffect, useRef, useState, type FormEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import type {
  Config,
  UploadDrift,
  OrgBearerStatus,
  ReleaseChannel,
  RsiCookieStatus,
  StatusResponse,
  SyncBacklog,
  SyncPreset,
  SyncStats,
  Theme,
} from '../api';
import {
  api,
  detectSyncPreset,
  RELEASE_CHANNEL_LABELS,
  SYNC_PRESETS,
  THEMES,
} from '../api';
import {
  applyUpdate,
  checkForUpdate,
  type UpdateCheckResult,
  type UpdateInfo,
} from '../updater';
import {
  Field,
  GhostButton,
  PrimaryButton,
  StatusDot,
  TextInput,
  TrayCard,
} from './tray/primitives';
import { AutostartToggle } from './AutostartToggle';
import { ReingestCard } from './ReingestCard';
import { ReparseCard } from './ReparseCard';
import { useFieldFocus } from '../hooks/useFieldFocus';
import { InlineCheck, type InlineCheckResult } from './InlineCheck';
import { friendlyError } from '../lib/friendlyError';
import { shouldShowChannelMismatchBanner } from './channelMismatch';
import { applyThemeWithWave } from '../lib/theme-transition';
import {
  DEFAULT_WAVE_SPEED,
  isWaveSpeed,
  WAVE_SPEEDS,
  type WaveSpeed,
} from '../lib/wave-speed';

/// Compact one-line presentation of a FriendlyError for inline span
/// usage. Keeps the title and body together; hint is dropped (we
/// already separately surface debug-logging hints elsewhere).
function inlineFriendly(err: unknown): string {
  const f = friendlyError(err);
  return `${f.title}: ${f.body}`;
}

/// Structural equality via JSON serialisation. Config is serde-derived
/// TOML so key order is stable and all values are serialisable — safe
/// for our purposes.
function isDeepEqual<T>(a: T, b: T): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

interface Props {
  config: Config;
  onSave: (next: Config) => Promise<void>;
  /**
   * Live status snapshot from the parent's polling hook. Drives the
   * Remote sync card's health pill (OK / ERR / IDLE / OFF). Optional
   * because the same component is also rendered in isolation by tests
   * that don't care about the health pill — those fall through to
   * IDLE / OFF as appropriate.
   */
  status?: StatusResponse | null;
}

type SyncHealth = 'ok' | 'err' | 'stale' | 'paused' | 'idle' | 'off';

/// Minimum staleness threshold in seconds when the worker has fired
/// at least once but hasn't attempted again recently. Set to 2× the
/// bulk interval, but never below this floor — protects against a
/// pathologically-low interval producing flapping pills.
const STALE_FLOOR_SECS = 60;

/// Derive the Remote sync card's health pill from the device's
/// pairing state, the per-config enabled flag, the latest `SyncStats`
/// snapshot from the polling hook, the bulk interval (for staleness
/// calculation), and the `paused` flag (set when the sync worker
/// emits `sync-paused`).
///
/// Precedence: OFF > PAUSED > IDLE > ERR > STALE > OK.
/// - OFF wins over everything — user disabled sync, don't nag.
/// - PAUSED next — worker bailed (auth_lost); needs re-pair.
/// - IDLE when no stats yet — never polled.
/// - ERR when there's a recorded error — we have an explanation,
///   show it. ERR beats STALE: a known failure is more useful
///   information than "we don't know".
/// - STALE when last_attempt_at is older than 2× the bulk interval
///   AND there's no error. Catches "looks connected but isn't
///   shipping" silent-failure modes (2026-05-28 outage: workers
///   alive, auth_lost guard skipping drain, sync_stats frozen on
///   its last green reading, pill happily showed OK for 10+ hours).
/// - OK only when we've had a recent success.
function deriveSyncHealth(
  isPaired: boolean,
  enabled: boolean,
  sync: SyncStats | null | undefined,
  bulkIntervalSecs: number,
  paused: boolean,
): SyncHealth {
  if (!isPaired || !enabled) return 'off';
  if (paused) return 'paused';
  if (!sync) return 'idle';
  if (sync.last_error) return 'err';
  if (sync.last_attempt_at) {
    const ageSecs =
      (Date.now() - Date.parse(sync.last_attempt_at)) / 1000;
    const staleThresholdSecs = Math.max(
      bulkIntervalSecs * 2,
      STALE_FLOOR_SECS,
    );
    if (Number.isFinite(ageSecs) && ageSecs > staleThresholdSecs) {
      return 'stale';
    }
  }
  if (sync.last_success_at) return 'ok';
  return 'idle';
}

const SYNC_HEALTH_LABEL: Record<SyncHealth, string> = {
  ok: 'OK',
  err: 'ERR',
  stale: 'STALE',
  paused: 'PAUSED',
  idle: 'IDLE',
  off: 'OFF',
};

const SYNC_HEALTH_TONE: Record<
  SyncHealth,
  'ok' | 'danger' | 'dim' | 'warn'
> = {
  ok: 'ok',
  err: 'danger',
  stale: 'warn',
  paused: 'warn',
  idle: 'dim',
  off: 'dim',
};

const SYNC_HEALTH_COLOR: Record<SyncHealth, string> = {
  ok: 'var(--ok)',
  err: 'var(--danger)',
  stale: 'var(--warn)',
  paused: 'var(--warn)',
  idle: 'var(--fg-dim)',
  off: 'var(--fg-dim)',
};

/** Labels for the Appearance card's Speed control. Mirrors the web
 * `WaveSpeedControl`'s `LABELS`. */
const WAVE_SPEED_LABEL: Readonly<Record<WaveSpeed, string>> = {
  off: 'Off',
  slow: 'Slow',
  normal: 'Normal',
  fast: 'Fast',
};

/**
 * Configuration UI for the tray client.
 *
 * Keeps form state local (uncontrolled-ish) until the user hits Save.
 * No optimistic mutation — the parent re-fetches after save lands so
 * we never display a value that the backend hasn't actually persisted.
 */
export function SettingsPane({ config, onSave, status }: Props) {
  const [draft, setDraft] = useState<Config>(config);
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [pairingCode, setPairingCode] = useState('');
  const [pairing, setPairing] = useState(false);
  const [pairError, setPairError] = useState<string | null>(null);
  const [pairedAs, setPairedAs] = useState<string | null>(null);

  const [cookieStatus, setCookieStatus] = useState<RsiCookieStatus | null>(
    null,
  );
  const [cookieDraft, setCookieDraft] = useState('');
  const [cookieSaving, setCookieSaving] = useState(false);
  const [cookieError, setCookieError] = useState<string | null>(null);
  const [cookieSavedAt, setCookieSavedAt] = useState<number | null>(null);

  // Org-connector bearer token — write-only, mirroring the RSI cookie
  // pattern above. The secret lives in the OS keychain (never in
  // `Config`), so it's never bound to `draft`: we probe status on mount
  // and manage it purely through the set/clear commands. The stored
  // value is never displayed — only a redacted preview tail.
  const [bearerStatus, setBearerStatus] = useState<OrgBearerStatus | null>(
    null,
  );
  const [bearerDraft, setBearerDraft] = useState('');
  const [bearerSaving, setBearerSaving] = useState(false);
  const [bearerError, setBearerError] = useState<string | null>(null);
  const [bearerSavedAt, setBearerSavedAt] = useState<number | null>(null);

  // Updates card state. `appVersion` is the Cargo workspace version
  // (e.g. "0.2.0-alpha") sourced via api.getAppVersion() so it
  // matches the GitHub release tag. Tauri's own getVersion() would
  // return the numeric tauri.conf.json value (MSI-friendly subset).
  // `updateState` drives the status
  // text/buttons; `installProgress` is non-null only while a download
  // is in flight.
  const [appVersion, setAppVersion] = useState<string | null>(null);
  // The compiled-in channel of the running binary (distinct from
  // draft.release_channel, which is the user-configured channel the
  // updater queries). Null while loading or if the command fails —
  // a missing build channel suppresses the mismatch banner since we
  // can't make a confident comparison.
  const [buildChannel, setBuildChannel] = useState<ReleaseChannel | null>(null);
  const [updateState, setUpdateState] = useState<
    | { kind: 'idle' }
    | { kind: 'checking' }
    | { kind: 'available'; info: UpdateInfo }
    | { kind: 'up_to_date' }
    | { kind: 'error'; message: string }
    | { kind: 'installing' }
  >({ kind: 'idle' });
  const [installProgress, setInstallProgress] = useState<{
    downloaded: number;
    total: number | null;
  } | null>(null);

  const [revoked, setRevoked] = useState(false);
  // `paused` mirrors a `sync-paused` emit from the Rust sync worker —
  // fired when the worker exits its loop because `auth_lost` is set.
  // The Settings pane uses it to drive the health pill to "PAUSED"
  // and surface an explanatory notice. Cleared on save/re-pair, same
  // shape as `revoked`. Surfaced 2026-05-28 after the silent
  // auth_lost-loop outage; previously the health pill stayed "OK"
  // forever because sync_stats never updated.
  const [paused, setPaused] = useState(false);

  // Unsaved-draft guard: tracks the remote config that arrived while
  // the user had uncommitted edits, and the last-saved baseline used
  // to detect whether the draft diverges from what was persisted.
  // Upload-queue depth. Polled rather than pushed: the drain has no
  // event of its own, and a 5 s COUNT(*) against a partial index is
  // cheaper than plumbing a new emit through both sync lanes.
  const [backlog, setBacklog] = useState<SyncBacklog | null>(null);
  const [uploading, setUploading] = useState(false);
  // Drift state. Manual only — never fetched on mount or on the status tick.
  const [drift, setDrift] = useState<UploadDrift | null>(null);
  const [checkingDrift, setCheckingDrift] = useState(false);
  const [requeueing, setRequeueing] = useState(false);
  const [driftError, setDriftError] = useState<string | null>(null);
  const [pendingRemote, setPendingRemote] = useState<Config | null>(null);
  const [savedBaseline, setSavedBaseline] = useState<Config>(config);

  // Refs so the prop-watching effect can read current draft / baseline
  // without subscribing to them as deps (which would re-fire the
  // effect on every keystroke and spuriously trip the "editing"
  // branch).
  const draftRef = useRef(draft);
  const baselineRef = useRef(savedBaseline);
  useEffect(() => { draftRef.current = draft; }, [draft]);
  useEffect(() => { baselineRef.current = savedBaseline; }, [savedBaseline]);

  // Field-focus registration for cross-pane HealthCard CTAs. Each
  // outer wrapper registers its DOM element; useFieldFocus.focus
  // scrolls + focuses the first interactive child.
  //
  // Stable-position fields use refs + a one-shot effect. Fields
  // inside conditional branches (e.g. pairing input, which is only
  // mounted when !isPaired) use ref callbacks so the registration
  // follows the mount/unmount of the branch.
  const fieldFocus = useFieldFocus();
  const gamelogPathRef = useRef<HTMLDivElement>(null);
  const apiUrlRef = useRef<HTMLDivElement>(null);
  const rsiCookieRef = useRef<HTMLDivElement>(null);
  const updatesRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    fieldFocus.register('gamelog_path', gamelogPathRef.current);
    fieldFocus.register('api_url', apiUrlRef.current);
    fieldFocus.register('rsi_cookie', rsiCookieRef.current);
    fieldFocus.register('updates', updatesRef.current);
  }, [fieldFocus]);

  const setPairingCodeNode = (el: HTMLDivElement | null) => {
    fieldFocus.register('pairing_code', el);
  };

  useEffect(() => {
    let cancelled = false;
    api
      .getRsiCookieStatus()
      .then((next) => {
        if (!cancelled) setCookieStatus(next);
      })
      .catch((e) => {
        if (!cancelled) setCookieError(inlineFriendly(e));
      });
    api
      .getOrgBearerStatus()
      .then((next) => {
        if (!cancelled) setBearerStatus(next);
      })
      .catch((e) => {
        if (!cancelled) setBearerError(inlineFriendly(e));
      });
    api
      .getAppVersion()
      .then((v) => {
        if (!cancelled) setAppVersion(v);
      })
      .catch(() => {
        // Version is informational; if it fails we just don't show it.
      });
    api
      .getBuildReleaseChannel()
      .then((c) => {
        if (!cancelled) setBuildChannel(c);
      })
      .catch(() => {
        // Build channel is informational; without it we just don't
        // render the mismatch banner or the channel in the kicker.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen('sync-revoked', () => setRevoked(true)).then((unl) => {
      // Unmounted before listen() resolved → detach now, else the listener
      // leaks (SettingsPane remounts on every tab switch). M-U5.
      if (cancelled) unl();
      else unlisten = unl;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen('sync-paused', () => setPaused(true)).then((unl) => {
      if (cancelled) unl();
      else unlisten = unl;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Poll the upload queue while the Settings pane is mounted. 5 s is
  // fast enough that a draining backlog visibly ticks down (the whole
  // point of the readout) and slow enough to be free next to the
  // pane's other work. A failure leaves the last-known value rather
  // than blanking the card — a transient DB lock must not read as
  // "queue empty".
  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const next = await api.getSyncBacklog();
        if (!cancelled) setBacklog(next);
      } catch {
        // Keep the previous reading; the next tick retries.
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 5000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  // The App owns the canonical Config and subscribes to the bulk-lane
  // `config-changed` event there (so it fires regardless of which tab
  // the user is on). When the parent replaces the config — whether
  // from a remote download, a Save round-trip, or any other source —
  // we mirror it into draft+baseline iff there are no unsaved local
  // edits. Otherwise stash the incoming config and surface the
  // reload/keep banner so the user picks rather than getting their
  // half-typed edits clobbered.
  useEffect(() => {
    const editing = !isDeepEqual(draftRef.current, baselineRef.current);
    if (editing) {
      setPendingRemote(config);
    } else {
      setDraft(config);
      setSavedBaseline(config);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config]);

  // Kick both sync lanes so the queue starts draining immediately
  // instead of waiting out the current interval. The button is a
  // nudge, not a separate upload path — the same worker does the work,
  // which is why there is no progress bar here (the queue count IS the
  // progress bar).
  const handleUploadNow = async () => {
    setUploading(true);
    try {
      await api.retrySyncNow();
      // Give the woken lane a moment to ship its first page so the
      // count visibly moves before the button re-enables.
      await new Promise((r) => setTimeout(r, 1200));
      setBacklog(await api.getSyncBacklog());
    } catch {
      // Non-fatal: the worker drains on its own schedule regardless.
    } finally {
      setUploading(false);
    }
  };

  const handleCheckDrift = async () => {
    setCheckingDrift(true);
    setDriftError(null);
    try {
      setDrift(await api.checkUploadDrift());
    } catch (e) {
      setDriftError(inlineFriendly(e));
      setDrift(null);
    } finally {
      setCheckingDrift(false);
    }
  };

  const handleRequeueMissing = async () => {
    if (!drift) return;
    const types = drift.rows.filter((r) => r.missing > 0).map((r) => r.event_type);
    if (types.length === 0) return;
    setRequeueing(true);
    setDriftError(null);
    try {
      await api.requeueMissingEvents(types);
      // Re-check so the panel reflects reality rather than intent, and
      // refresh the queue card which now has work to do.
      setBacklog(await api.getSyncBacklog());
      setDrift(await api.checkUploadDrift());
    } catch (e) {
      setDriftError(inlineFriendly(e));
    } finally {
      setRequeueing(false);
    }
  };

  // Manual "Check for updates" handler. Bypasses the auto-check
  // preference — pressing the button always checks, regardless of the
  // toggle state, because that's the user's explicit intent.
  const handleCheckForUpdate = async () => {
    setUpdateState({ kind: 'checking' });
    try {
      const result: UpdateCheckResult = await checkForUpdate(
        draft.release_channel,
      );
      if (result.available) {
        setUpdateState({ kind: 'available', info: result });
      } else {
        setUpdateState({ kind: 'up_to_date' });
      }
    } catch (e) {
      setUpdateState({ kind: 'error', message: String(e) });
    }
  };

  // "Install and restart". The Rust install command re-checks the
  // channel before downloading, so a fresher release between
  // user-pressed-Install and now would simply install the newer
  // one — fine for our scale, and removes the need to plumb the
  // (non-Serializable) Update handle across the IPC bridge.
  const handleInstallUpdate = async () => {
    if (updateState.kind !== 'available') return;
    setUpdateState({ kind: 'installing' });
    setInstallProgress({ downloaded: 0, total: null });
    try {
      await applyUpdate(draft.release_channel, (downloaded, total) => {
        setInstallProgress({ downloaded, total });
      });
      // applyUpdate calls relaunch(); we never reach this line.
    } catch (e) {
      setInstallProgress(null);
      setUpdateState({ kind: 'error', message: String(e) });
    }
  };

  // Commit a channel-mismatch banner action immediately (Switch /
  // Dismiss) — these shouldn't wait for the main Save button. We
  // patch the LAST-SAVED baseline (not the draft) so the user's
  // unsaved edits elsewhere in the pane aren't accidentally
  // persisted along the way. Draft is then synced for any field
  // we touched so the form doesn't appear to "lose" the change.
  const commitChannelBannerAction = async (
    patch: Partial<Pick<Config, 'release_channel' | 'channel_mismatch_ack'>>,
  ) => {
    const next: Config = { ...savedBaseline, ...patch };
    try {
      await onSave(next);
      setSavedBaseline(next);
      // Keep the draft's other edits intact; only apply the
      // channel-specific keys we just persisted.
      setDraft((prev) => ({ ...prev, ...patch }));
      setError(null);
    } catch (err) {
      setError(inlineFriendly(err));
    }
  };

  // Wraps `setDraft` so any in-pane edit clears the trailing "✓ Saved"
  // pip and any save error — both are stale the moment the user
  // resumes editing.
  const editDraft = (mutate: (prev: Config) => Config) => {
    setDraft(mutate);
    setSavedAt(null);
    setError(null);
  };

  const updateRemote = (patch: Partial<Config['remote_sync']>) =>
    editDraft((prev) => ({
      ...prev,
      remote_sync: { ...prev.remote_sync, ...patch },
    }));

  const updateOrgConnector = (patch: Partial<Config['org_connector']>) =>
    editDraft((prev) => ({
      ...prev,
      org_connector: { ...prev.org_connector, ...patch },
    }));

  // Inline validation for the org-connector card. Mirrors the Rust-side
  // build_ws_url contract: non-loopback hosts must be TLS (https/wss);
  // plaintext (http/ws) is allowed only for localhost. Null when the
  // connector is off or the field is valid. Derived each render from
  // `draft` so the message clears as the user types a valid value.
  const orgConnectorUrlError: string | null = (() => {
    const oc = draft.org_connector;
    if (!oc.enabled) return null;
    const url = (oc.platform_url ?? '').trim();
    if (!url) return 'Enter the org platform URL.';
    let parsed: URL;
    try {
      parsed = new URL(url);
    } catch {
      return 'Enter a valid URL (https:// or wss://).';
    }
    const loopback = ['localhost', '127.0.0.1', '::1', '[::1]'].includes(
      parsed.hostname,
    );
    const scheme = parsed.protocol.replace(':', '');
    const tls = scheme === 'https' || scheme === 'wss';
    const plain = scheme === 'http' || scheme === 'ws';
    if (!tls && !plain) return 'URL must use https://, wss://, http:// or ws://.';
    if (!tls && !loopback)
      return 'Non-localhost URLs must use https:// or wss://.';
    return null;
  })();

  // The token no longer lives in `draft` (it's in the keychain), so
  // required-ness is derived from the keychain status probe. While the
  // probe is in flight (`bearerStatus === null`) we don't assert the
  // error — avoids a flash of "required" before status loads.
  const orgConnectorTokenError: string | null =
    draft.org_connector.enabled &&
    bearerStatus !== null &&
    !bearerStatus.configured
      ? 'A bearer token is required when the connector is on.'
      : null;

  // Eager preview: run the theme-wave sweep immediately so the user
  // sees (and feels) the switch before Save — `applyThemeWithWave`
  // stamps `[data-theme]` itself (instant or animated depending on
  // reduced-motion / the resolved wave speed) and `onPersist` mirrors
  // the choice into `draft` the same way the old instant write did.
  // App.tsx's boot `useEffect` will reconcile on persistence and is
  // deliberately NOT wave-animated (see its comment) — only this
  // user-initiated path replays the sweep. If the user dismisses
  // without saving, the preview persists until next config refresh —
  // a deliberate UX trade for instant feedback, unchanged from before.
  const updateTheme = (theme: Theme) => {
    applyThemeWithWave(theme, {
      onPersist: (next) => {
        editDraft((prev) => ({ ...prev, theme: next }));
      },
    });
  };

  // Speed control has no wave of its own to preview (nothing to
  // replay until the next theme switch, mirroring the web
  // `WaveSpeedControl` behaviour) — just stamp the attribute so the
  // NEXT `applyThemeWithWave` call picks up the new duration, and
  // mirror the choice into `draft` for Save.
  const updateWaveSpeed = (speed: WaveSpeed) => {
    document.documentElement.dataset.waveSpeed = speed;
    editDraft((prev) => ({ ...prev, theme_wave_speed: speed }));
  };

  const handlePair = async () => {
    if (!draft.remote_sync.api_url) {
      setPairError('Set the API URL above first.');
      return;
    }
    setPairing(true);
    setPairError(null);
    try {
      const outcome = await api.pairDevice(
        draft.remote_sync.api_url,
        pairingCode,
      );
      // Reload from disk — the Rust side already persisted token +
      // claimed_handle, no point trusting our in-memory draft.
      const fresh = await api.getConfig();
      setDraft(fresh);
      setPairingCode('');
      setPairedAs(outcome.claimed_handle);
    } catch (err) {
      setPairError(inlineFriendly(err));
    } finally {
      setPairing(false);
    }
  };

  const handleSaveCookie = async () => {
    if (!cookieDraft.trim()) {
      setCookieError('Paste the cookie value first.');
      return;
    }
    setCookieSaving(true);
    setCookieError(null);
    try {
      const next = await api.setRsiCookie(cookieDraft.trim());
      setCookieStatus(next);
      setCookieDraft('');
      setCookieSavedAt(Date.now());
    } catch (err) {
      setCookieError(inlineFriendly(err));
    } finally {
      setCookieSaving(false);
    }
  };

  const handleClearCookie = async () => {
    if (
      !window.confirm(
        'Clear the stored RSI cookie? Hangar refresh will pause until you paste a new one.',
      )
    ) {
      return;
    }
    setCookieSaving(true);
    setCookieError(null);
    try {
      const next = await api.clearRsiCookie();
      setCookieStatus(next);
      setCookieSavedAt(null);
    } catch (err) {
      setCookieError(inlineFriendly(err));
    } finally {
      setCookieSaving(false);
    }
  };

  const handleSaveBearer = async () => {
    if (!bearerDraft.trim()) {
      setBearerError('Paste the bearer token first.');
      return;
    }
    setBearerSaving(true);
    setBearerError(null);
    try {
      const next = await api.setOrgBearer(bearerDraft.trim());
      setBearerStatus(next);
      setBearerDraft('');
      setBearerSavedAt(Date.now());
    } catch (err) {
      setBearerError(inlineFriendly(err));
    } finally {
      setBearerSaving(false);
    }
  };

  const handleClearBearer = async () => {
    if (
      !window.confirm(
        'Clear the stored org connector token? The connector will pause until you paste a new one.',
      )
    ) {
      return;
    }
    setBearerSaving(true);
    setBearerError(null);
    try {
      const next = await api.clearOrgBearer();
      setBearerStatus(next);
      setBearerSavedAt(null);
    } catch (err) {
      setBearerError(inlineFriendly(err));
    } finally {
      setBearerSaving(false);
    }
  };

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    setError(null);
    try {
      await onSave(draft);
      setSavedBaseline(draft);
      setSavedAt(Date.now());
    } catch (err) {
      setError(inlineFriendly(err));
    } finally {
      setSaving(false);
    }
  };

  // Paired state derives from claimed_handle alone. Since M-T6 the device JWT
  // (access_token) is #[serde(skip)] — it lives in the OS keychain and is
  // never sent to the UI, so it's always null here. claimed_handle and the
  // keychain token are set/cleared together (redeem_pair / unpair), so the
  // handle is a faithful proxy. (Checking access_token here made every paired
  // device read as unpaired after a remount.)
  const isPaired = Boolean(draft.remote_sync.claimed_handle);

  // Drive the Remote sync card's right-slot pill (OK / ERR / IDLE /
  // OFF) and the inner "Paired as" dot off the live `SyncStats`
  // snapshot from the parent's polling hook. Falls through to IDLE
  // when no status has been polled yet — the dot stays neutral
  // rather than misrepresenting health as "ok" the way the old
  // hardcoded indicator did.
  const syncHealth = deriveSyncHealth(
    isPaired,
    draft.remote_sync.enabled,
    status?.sync,
    draft.remote_sync.interval_secs,
    paused,
  );

  return (
    <form
      onSubmit={handleSubmit}
      style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
    >
      <TrayCard
        title="Unreadable log entries"
        kicker={draft.share_unknown_tags ? 'on' : 'off'}
      >
        <p
          style={{
            margin: '0 0 10px',
            color: 'var(--fg-muted)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          Off by default. Sends the names of log entry types StarStats
          couldn&apos;t read &mdash; never the log lines themselves. Helps
          spot when a game update breaks tracking.
        </p>
        <label
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            fontSize: 12,
            cursor: isPaired ? 'pointer' : 'not-allowed',
            opacity: isPaired ? 1 : 0.5,
          }}
        >
          <input
            type="checkbox"
            checked={draft.share_unknown_tags}
            disabled={!isPaired}
            onChange={(e) => {
              editDraft((prev) => ({
                ...prev,
                share_unknown_tags: e.target.checked,
              }));
            }}
            style={{ accentColor: 'var(--accent)' }}
            aria-label="Report unreadable log entry names"
          />
          <span>
            <strong style={{ color: 'var(--fg)' }}>
              Report unreadable log entry names
            </strong>
            {!isPaired && (
              <span
                style={{ display: 'block', color: 'var(--fg-dim)', fontSize: 11 }}
              >
                Pair this tray (Remote sync card below) to enable.
              </span>
            )}
          </span>
        </label>
      </TrayCard>

      <TrayCard
        title="Cloud sync"
        kicker={draft.sync_with_cloud ? 'on' : 'off'}
      >
        <p
          style={{
            margin: '0 0 10px',
            color: 'var(--fg-muted)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          Off by default. When on, this tray's theme and settings are stored
          on your account and follow you to other devices. You can revoke
          sync from any uplink on the Connected Uplinks page on the web.
        </p>
        <label
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            fontSize: 12,
            cursor: isPaired ? 'pointer' : 'not-allowed',
            opacity: isPaired ? 1 : 0.5,
          }}
        >
          <input
            type="checkbox"
            checked={draft.sync_with_cloud}
            disabled={!isPaired}
            onChange={(e) => {
              editDraft((prev) => ({ ...prev, sync_with_cloud: e.target.checked }));
              if (e.target.checked) {
                setRevoked(false);
                setPaused(false);
              }
            }}
            style={{ accentColor: 'var(--accent)' }}
            aria-label="Sync settings with your account"
          />
          <span>
            <strong style={{ color: 'var(--fg)' }}>
              Sync settings with your account
            </strong>
            {!isPaired && (
              <span style={{ display: 'block', color: 'var(--fg-dim)', fontSize: 11 }}>
                Pair this tray (Remote sync card below) to enable.
              </span>
            )}
          </span>
        </label>
        {revoked && (
          <p
            style={{
              margin: '8px 0 0',
              padding: '8px 10px',
              background: 'var(--bg-elev)',
              border: '1px solid var(--warn)',
              borderRadius: 'var(--r-sm)',
              fontSize: 12,
              color: 'var(--warn)',
            }}
            role="status"
          >
            Cloud sync is disabled for this uplink. Toggle Cloud sync back on to resume.
          </p>
        )}
        {paused && (
          <p
            style={{
              margin: '8px 0 0',
              padding: '8px 10px',
              background: 'var(--bg-elev)',
              border: '1px solid var(--warn)',
              borderRadius: 'var(--r-sm)',
              fontSize: 12,
              color: 'var(--warn)',
            }}
            role="status"
          >
            Sync paused: the server rejected this uplink's token. Re-pair the
            device (Connected Uplinks card below) to resume.
          </p>
        )}
      </TrayCard>

      <TrayCard
        title="Appearance"
        kicker={`theme · ${draft.theme}`}
      >
        <p
          style={{
            margin: '0 0 10px',
            color: 'var(--fg-muted)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          Four themes from the StarStats design system. Swatches preview
          live; the change persists on Save.
        </p>
        <div
          role="radiogroup"
          aria-label="Theme"
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(4, 1fr)',
            gap: 8,
          }}
        >
          {THEMES.map((t) => {
            const active = draft.theme === t.id;
            return (
              <button
                key={t.id}
                type="button"
                role="radio"
                aria-checked={active}
                onClick={() => updateTheme(t.id)}
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'stretch',
                  gap: 6,
                  padding: 8,
                  background: t.swatch.surface,
                  border: `1px solid ${active ? 'var(--accent)' : 'var(--border-strong)'}`,
                  borderRadius: 'var(--r-sm)',
                  cursor: 'pointer',
                  outline: active ? '2px solid var(--accent)' : 'none',
                  outlineOffset: 1,
                  transition:
                    'transform 120ms var(--ease-out), border-color 180ms var(--ease-out)',
                  fontFamily: 'inherit',
                  textAlign: 'left',
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    display: 'flex',
                    gap: 3,
                    height: 16,
                    borderRadius: 2,
                    overflow: 'hidden',
                  }}
                >
                  <span style={{ flex: 1, background: t.swatch.bg }} />
                  <span style={{ flex: 1, background: t.swatch.surface }} />
                  <span style={{ flex: 1, background: t.swatch.accent }} />
                  <span style={{ flex: 1, background: t.swatch.fg }} />
                </span>
                <span
                  style={{
                    fontSize: 11,
                    fontWeight: 600,
                    color: t.swatch.fg,
                    letterSpacing: '0.04em',
                  }}
                >
                  {t.label}
                </span>
                <span
                  style={{
                    fontSize: 10,
                    color: t.swatch.fg,
                    opacity: 0.55,
                    fontFamily: 'var(--font-mono)',
                  }}
                >
                  {t.tagline}
                </span>
              </button>
            );
          })}
        </div>

        <p
          style={{
            margin: '16px 0 8px',
            color: 'var(--fg-muted)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          Speed of the sweep animation when switching themes. Off skips
          the animation entirely.
        </p>
        <div
          role="radiogroup"
          aria-label="Theme-switch wave speed"
          style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}
        >
          {WAVE_SPEEDS.map((speed) => {
            const active =
              (isWaveSpeed(draft.theme_wave_speed)
                ? draft.theme_wave_speed
                : DEFAULT_WAVE_SPEED) === speed;
            return (
              <button
                key={speed}
                type="button"
                role="radio"
                aria-checked={active}
                onClick={() => updateWaveSpeed(speed)}
                style={{
                  padding: '6px 12px',
                  background: active ? 'var(--surface-3)' : 'var(--surface-2)',
                  border: `1px solid ${active ? 'var(--accent)' : 'var(--border-strong)'}`,
                  borderRadius: 'var(--r-sm)',
                  cursor: 'pointer',
                  fontFamily: 'inherit',
                  fontSize: 12,
                  fontWeight: active ? 600 : 400,
                  color: 'var(--fg)',
                }}
              >
                {WAVE_SPEED_LABEL[speed]}
              </button>
            );
          })}
        </div>
      </TrayCard>

      <div ref={gamelogPathRef}>
        <TrayCard title="Game.log">
          <Field
            label="Override path"
            hint="Leave blank to auto-discover the largest LIVE/PTU/EPTU log."
          >
            <TextInput
              type="text"
              value={draft.gamelog_path ?? ''}
              placeholder="auto-discover"
              onChange={(e) =>
                editDraft((prev) => ({
                  ...prev,
                  gamelog_path: e.target.value || null,
                }))
              }
              spellCheck={false}
            />
          </Field>
        </TrayCard>
      </div>

      <ReingestCard />

      <ReparseCard />

      <div ref={updatesRef}>
      <TrayCard
        title="Updates"
        kicker={
          appVersion
            ? buildChannel
              ? `${RELEASE_CHANNEL_LABELS[buildChannel].toLowerCase()} · v${appVersion}`
              : `v${appVersion}`
            : undefined
        }
        right={
          <label
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              fontSize: 11,
              color: 'var(--fg-muted)',
              cursor: 'pointer',
            }}
            title="Check for updates automatically when the app launches"
          >
            <input
              type="checkbox"
              checked={draft.auto_update_check}
              onChange={(e) =>
                editDraft((prev) => ({
                  ...prev,
                  auto_update_check: e.target.checked,
                }))
              }
              style={{ accentColor: 'var(--accent)' }}
            />
            <span style={{ textTransform: 'uppercase', letterSpacing: '0.1em' }}>
              auto
            </span>
          </label>
        }
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {buildChannel &&
            shouldShowChannelMismatchBanner(
              buildChannel,
              draft.release_channel,
              savedBaseline.channel_mismatch_ack,
            ) && (
              <div
                role="status"
                aria-label="Channel mismatch"
                style={{
                  padding: '8px 10px',
                  background: 'var(--bg-elev)',
                  border: '1px solid var(--accent)',
                  borderRadius: 'var(--r-sm)',
                  fontSize: 12,
                  lineHeight: 1.45,
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 8,
                }}
              >
                <strong
                  style={{
                    fontSize: 11,
                    color: 'var(--accent)',
                    textTransform: 'uppercase',
                    letterSpacing: '0.08em',
                  }}
                >
                  Channel mismatch
                </strong>
                <span style={{ color: 'var(--fg-muted)' }}>
                  You're running a {RELEASE_CHANNEL_LABELS[buildChannel]} build
                  but configured to receive{' '}
                  {RELEASE_CHANNEL_LABELS[draft.release_channel]} updates. The
                  next update check will poll the{' '}
                  {RELEASE_CHANNEL_LABELS[draft.release_channel].toLowerCase()}{' '}
                  manifest.
                </span>
                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                  <PrimaryButton
                    type="button"
                    onClick={() =>
                      void commitChannelBannerAction({
                        release_channel: buildChannel,
                      })
                    }
                  >
                    Switch to {RELEASE_CHANNEL_LABELS[buildChannel]}
                  </PrimaryButton>
                  <GhostButton
                    type="button"
                    onClick={() =>
                      void commitChannelBannerAction({
                        channel_mismatch_ack: buildChannel,
                      })
                    }
                  >
                    Dismiss
                  </GhostButton>
                </div>
              </div>
            )}
          <UpdateStatusLine state={updateState} progress={installProgress} />
          <label
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              fontSize: 12,
              color: 'var(--fg-muted)',
            }}
            title="Switch channels at any time. The next check polls the new channel's manifest."
          >
            <span style={{ minWidth: 64 }}>Channel</span>
            <select
              value={draft.release_channel}
              onChange={(e) =>
                editDraft((prev) => ({
                  ...prev,
                  release_channel: e.target.value as ReleaseChannel,
                }))
              }
              disabled={
                updateState.kind === 'checking' ||
                updateState.kind === 'installing'
              }
              style={{
                // Match the INPUT_BASE pattern from primitives.tsx so
                // this dropdown sits visually next to the TextInputs
                // elsewhere on the same card — same surface, radius,
                // font, and padding contract. The native chrome of
                // the OPTIONS popup is still browser-controlled
                // (can't be themed reliably), but the closed-state
                // control now reads as part of the design system.
                background: 'var(--bg)',
                color: 'var(--fg)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-sm)',
                padding: '7px 9px',
                fontFamily: 'var(--font-mono)',
                fontSize: 12,
                cursor:
                  updateState.kind === 'checking' ||
                  updateState.kind === 'installing'
                    ? 'not-allowed'
                    : 'pointer',
              }}
            >
              {(Object.keys(RELEASE_CHANNEL_LABELS) as ReleaseChannel[]).map(
                (ch) => (
                  <option key={ch} value={ch}>
                    {RELEASE_CHANNEL_LABELS[ch]}
                  </option>
                ),
              )}
            </select>
          </label>
          {updateState.kind === 'available' && updateState.info.notes && (
            <pre
              style={{
                margin: 0,
                padding: '8px 10px',
                background: 'var(--bg-elev)',
                border: '1px solid var(--border)',
                borderRadius: 4,
                fontSize: 11,
                lineHeight: 1.5,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-word',
                maxHeight: 160,
                overflowY: 'auto',
                color: 'var(--fg-muted)',
              }}
            >
              {updateState.info.notes}
            </pre>
          )}
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            {updateState.kind === 'available' ? (
              <PrimaryButton
                type="button"
                onClick={handleInstallUpdate}
                disabled={false}
              >
                Install v{updateState.info.version} and restart
              </PrimaryButton>
            ) : (
              <GhostButton
                type="button"
                onClick={handleCheckForUpdate}
                disabled={
                  updateState.kind === 'checking' ||
                  updateState.kind === 'installing'
                }
              >
                {updateState.kind === 'checking'
                  ? 'Checking…'
                  : 'Check for updates'}
              </GhostButton>
            )}
          </div>
          <label
            style={{
              display: 'flex',
              alignItems: 'flex-start',
              gap: 8,
              fontSize: 12,
              color: 'var(--fg-muted)',
              cursor: 'pointer',
              marginTop: 4,
              borderTop: '1px solid var(--border)',
              paddingTop: 10,
            }}
          >
            <input
              type="checkbox"
              checked={draft.debug_logging}
              onChange={(e) =>
                editDraft((prev) => ({
                  ...prev,
                  debug_logging: e.target.checked,
                }))
              }
              style={{ accentColor: 'var(--accent)', marginTop: 2 }}
            />
            <span style={{ lineHeight: 1.4 }}>
              <strong style={{ color: 'var(--fg)' }}>Debug logging</strong>
              <span style={{ display: 'block', fontSize: 11 }}>
                Writes a daily client.log to the user data dir for bug
                reports. Off by default. Restart after toggling.
              </span>
            </span>
          </label>
          <label
            style={{
              display: 'flex',
              alignItems: 'flex-start',
              gap: 8,
              fontSize: 12,
              color: 'var(--fg-muted)',
              cursor: 'pointer',
              marginTop: 4,
              borderTop: '1px solid var(--border)',
              paddingTop: 10,
            }}
          >
            <input
              type="checkbox"
              checked={draft.parser_enable_v2_metadata}
              onChange={(e) =>
                editDraft((prev) => ({
                  ...prev,
                  parser_enable_v2_metadata: e.target.checked,
                }))
              }
              style={{ accentColor: 'var(--accent)', marginTop: 2 }}
            />
            <span style={{ lineHeight: 1.4 }}>
              <strong style={{ color: 'var(--fg)' }}>
                Capture unrecognised log lines
              </strong>
              <span style={{ display: 'block', fontSize: 11 }}>
                Collects privacy-safe &ldquo;shapes&rdquo; of game.log lines
                StarStats can&rsquo;t parse yet into a local review queue, so
                you can submit them to help add new parser rules. Local-only
                until you choose to submit. On by default; turn it off here
                anytime. Restart after toggling.
              </span>
            </span>
          </label>
          <AutostartToggle />
        </div>
      </TrayCard>
      </div>

      <TrayCard
        title="Remote sync"
        right={
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
            }}
          >
            <span
              role="status"
              aria-label="Remote sync health"
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                fontSize: 11,
                color: SYNC_HEALTH_COLOR[syncHealth],
              }}
            >
              <StatusDot tone={SYNC_HEALTH_TONE[syncHealth]} />
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  textTransform: 'uppercase',
                  letterSpacing: '0.08em',
                }}
              >
                {SYNC_HEALTH_LABEL[syncHealth]}
              </span>
            </span>
            <label
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                fontSize: 11,
                color: 'var(--fg-muted)',
                cursor: 'pointer',
              }}
            >
              <input
                type="checkbox"
                checked={draft.remote_sync.enabled}
                onChange={(e) => updateRemote({ enabled: e.target.checked })}
                style={{ accentColor: 'var(--accent)' }}
              />
              <span
                style={{ textTransform: 'uppercase', letterSpacing: '0.1em' }}
              >
                {draft.remote_sync.enabled ? 'ON' : 'OFF'}
              </span>
            </label>
          </div>
        }
      >
        <p
          style={{
            margin: '0 0 12px',
            color: 'var(--fg-muted)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          Push events to a StarStats API server. Disabled by default — you
          choose when to share.
        </p>

        <fieldset
          disabled={!draft.remote_sync.enabled}
          style={{
            border: 'none',
            margin: 0,
            padding: 0,
            opacity: draft.remote_sync.enabled ? 1 : 0.45,
            display: 'flex',
            flexDirection: 'column',
            gap: 12,
          }}
        >
          <div ref={apiUrlRef}>
            <Field label="API URL">
              <TextInput
                type="url"
                value={draft.remote_sync.api_url ?? ''}
                placeholder="https://api.example.com"
                onChange={(e) =>
                  updateRemote({ api_url: e.target.value || null })
                }
                spellCheck={false}
              />
            </Field>
            <InlineCheck
              label="Test connection"
              value={draft.remote_sync.api_url ?? ''}
              onCheck={async (url): Promise<InlineCheckResult> => {
                const r = await api.checkApiUrl(url);
                return {
                  ok: r.ok,
                  message: r.ok
                    ? `Reachable${r.server_version ? ` · server v${r.server_version}` : ''}`
                    : r.error ?? 'Unknown error',
                };
              }}
            />
          </div>

          <Field label="Pairing">
            {isPaired ? (
              <div
                ref={setPairingCodeNode}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 12,
                  padding: '8px 10px',
                  background: 'var(--surface-2)',
                  border: '1px solid var(--border)',
                  borderRadius: 'var(--r-sm)',
                }}
              >
                <div
                  style={{ display: 'flex', alignItems: 'center', gap: 10 }}
                >
                  <StatusDot tone={SYNC_HEALTH_TONE[syncHealth]} />
                  <div>
                    <div style={{ fontSize: 12, color: 'var(--fg)' }}>
                      Paired as{' '}
                      <strong style={{ color: 'var(--accent)' }}>
                        {draft.remote_sync.claimed_handle}
                      </strong>
                    </div>
                    <div
                      style={{
                        fontSize: 11,
                        color: 'var(--fg-dim)',
                        fontFamily: 'var(--font-mono)',
                      }}
                    >
                      Device token stored in your OS keychain
                    </div>
                  </div>
                </div>
                <GhostButton
                  type="button"
                  onClick={() => {
                    // Clear both the persisted credentials and any stale
                    // form state — otherwise the previous success/error
                    // message hangs around when the pairing input
                    // re-renders.
                    updateRemote({
                      access_token: null,
                      claimed_handle: null,
                    });
                    setPairingCode('');
                    setPairError(null);
                    setPairedAs(null);
                  }}
                >
                  Unpair
                </GhostButton>
                <small style={{ fontSize: 11, color: 'var(--fg-dim)' }}>
                  Unpairing takes effect when you save your settings.
                </small>
              </div>
            ) : (
              <div
                ref={setPairingCodeNode}
                style={{ display: 'flex', flexDirection: 'column', gap: 8 }}
              >
                <small
                  style={{
                    fontSize: 11,
                    color: 'var(--fg-dim)',
                    lineHeight: 1.4,
                  }}
                >
                  Generate a pairing code on the StarStats website
                  (Connected Uplinks → Pair this tray) and type it below.
                </small>
                <div style={{ display: 'flex', gap: 8 }}>
                  <TextInput
                    type="text"
                    value={pairingCode}
                    placeholder="ABCDEFGH"
                    maxLength={8}
                    onChange={(e) =>
                      setPairingCode(e.target.value.toUpperCase())
                    }
                    spellCheck={false}
                    autoComplete="off"
                    style={{
                      flex: 1,
                      letterSpacing: '0.25em',
                      textAlign: 'center',
                      fontWeight: 600,
                      fontSize: 14,
                    }}
                  />
                  <PrimaryButton
                    type="button"
                    onClick={handlePair}
                    disabled={pairing || pairingCode.length !== 8}
                  >
                    {pairing ? 'Pairing…' : 'Pair'}
                  </PrimaryButton>
                </div>
                {pairError && (
                  <small style={{ fontSize: 12, color: 'var(--danger)' }}>
                    {pairError}
                  </small>
                )}
                {pairedAs && (
                  <small style={{ fontSize: 12, color: 'var(--ok)' }}>
                    ✓ Paired as {pairedAs}
                  </small>
                )}
              </div>
            )}
          </Field>

          <SyncSpeedSection
            draft={draft}
            updateRemote={updateRemote}
          />

          <div
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr 1fr 1fr',
              gap: 10,
            }}
          >
            <Field label="Priority interval">
              <div
                style={{ display: 'flex', alignItems: 'center', gap: 6 }}
              >
                <TextInput
                  type="number"
                  min={1}
                  max={60}
                  value={draft.remote_sync.priority_interval_secs}
                  onChange={(e) =>
                    updateRemote({
                      priority_interval_secs: Math.max(
                        1,
                        Number(e.target.value) || 5,
                      ),
                    })
                  }
                  style={{ flex: 1 }}
                />
                <span style={{ fontSize: 11, color: 'var(--fg-dim)' }}>
                  sec
                </span>
              </div>
            </Field>
            <Field label="Bulk interval">
              <div
                style={{ display: 'flex', alignItems: 'center', gap: 6 }}
              >
                <TextInput
                  type="number"
                  min={5}
                  max={3600}
                  value={draft.remote_sync.interval_secs}
                  onChange={(e) =>
                    updateRemote({
                      interval_secs: Math.max(
                        5,
                        Number(e.target.value) || 60,
                      ),
                    })
                  }
                  style={{ flex: 1 }}
                />
                <span style={{ fontSize: 11, color: 'var(--fg-dim)' }}>
                  sec
                </span>
              </div>
            </Field>
            <Field label="Batch size">
              <TextInput
                type="number"
                min={1}
                max={20000}
                value={draft.remote_sync.batch_size}
                onChange={(e) =>
                  updateRemote({
                    batch_size: Math.max(1, Number(e.target.value) || 200),
                  })
                }
              />
            </Field>
          </div>

          <label
            style={{
              display: 'flex',
              alignItems: 'flex-start',
              gap: 8,
              marginTop: 10,
              fontSize: 12,
            }}
          >
            <input
              type="checkbox"
              checked={draft.remote_sync.catch_up_enabled}
              onChange={(e) =>
                updateRemote({ catch_up_enabled: e.target.checked })
              }
              style={{ accentColor: 'var(--accent)', marginTop: 2 }}
              aria-label="Catch up on backlogs"
            />
            <span>
              Catch up on backlogs
              <small
                style={{
                  display: 'block',
                  color: 'var(--fg-dim)',
                  lineHeight: 1.4,
                }}
              >
                Upload back-to-back until the queue is empty instead of one
                batch per interval, using the larger catch-up batch size
                below. Paced down automatically while Star Citizen is
                running.
              </small>
            </span>
          </label>

          {draft.remote_sync.catch_up_enabled && (
            <div style={{ marginTop: 8, maxWidth: 200 }}>
              <Field label="Catch-up batch size">
                <TextInput
                  type="number"
                  min={1}
                  max={20000}
                  value={draft.remote_sync.catch_up_batch_size}
                  onChange={(e) =>
                    updateRemote({
                      catch_up_batch_size: Math.max(
                        1,
                        Number(e.target.value) || 2000,
                      ),
                    })
                  }
                />
              </Field>
            </div>
          )}

          <div style={{ marginTop: 10 }}>
            <UploadQueueSection
              backlog={backlog}
              onUploadNow={handleUploadNow}
              uploading={uploading}
            />
            <DriftSection
              drift={drift}
              checking={checkingDrift}
              requeueing={requeueing}
              error={driftError}
              onCheck={handleCheckDrift}
              onRequeue={handleRequeueMissing}
            />
          </div>
        </fieldset>
      </TrayCard>

      <TrayCard
        title="RSI session cookie"
        right={
          <span
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              fontSize: 11,
            }}
          >
            <StatusDot tone={cookieStatus?.configured ? 'ok' : 'warn'} />
            <span
              style={{
                color: cookieStatus?.configured
                  ? 'var(--ok)'
                  : 'var(--warn)',
                fontFamily: 'var(--font-mono)',
                textTransform: 'uppercase',
                letterSpacing: '0.08em',
              }}
            >
              {cookieStatus?.configured ? 'SET' : 'MISSING'}
            </span>
          </span>
        }
      >
        <p
          style={{
            margin: '0 0 12px',
            color: 'var(--fg-muted)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          {cookieStatus === null ? (
            'Loading…'
          ) : cookieStatus.configured ? (
            <>
              Configured (last 4 chars:{' '}
              <code
                style={{
                  color: 'var(--accent)',
                  fontFamily: 'var(--font-mono)',
                }}
              >
                {cookieStatus.preview ?? '????'}
              </code>
              ). Paste a new value to rotate.
            </>
          ) : (
            'Not configured. Paste your Rsi-Token cookie below.'
          )}
        </p>

        <div ref={rsiCookieRef}>
          <Field
            label="Rsi-Token cookie"
            hint="Find this in DevTools → Application → Cookies → robertsspaceindustries.com → Rsi-Token. Never leaves your machine — only parsed ship lists are sent."
          >
            <TextInput
              type="password"
              value={cookieDraft}
              placeholder="•••••••••••••••••••••••••••"
              onChange={(e) => {
                setCookieDraft(e.target.value);
                setCookieSavedAt(null);
                setCookieError(null);
              }}
              spellCheck={false}
              autoComplete="off"
            />
          </Field>
          <InlineCheck
            label="Test cookie"
            value={cookieDraft}
            onCheck={async (cookie): Promise<InlineCheckResult> => {
              const r = await api.checkRsiCookie(cookie);
              return {
                ok: r.ok,
                message: r.ok
                  ? r.handle
                    ? `Authenticated as ${r.handle}`
                    : 'Cookie accepted'
                  : r.error ?? 'Unknown error',
              };
            }}
          />
        </div>

        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            marginTop: 10,
          }}
        >
          <PrimaryButton
            type="button"
            onClick={handleSaveCookie}
            disabled={cookieSaving || !cookieDraft.trim()}
          >
            {cookieSaving ? 'Saving…' : 'Save cookie'}
          </PrimaryButton>
          <GhostButton
            type="button"
            onClick={handleClearCookie}
            disabled={cookieSaving || !cookieStatus?.configured}
          >
            Clear
          </GhostButton>
          {cookieSavedAt && !cookieSaving && !cookieError && (
            <span style={{ fontSize: 11, color: 'var(--ok)' }}>✓ Saved</span>
          )}
        </div>
        {cookieError && (
          <small
            style={{
              display: 'block',
              marginTop: 6,
              fontSize: 12,
              color: 'var(--danger)',
            }}
          >
            {cookieError}
          </small>
        )}
      </TrayCard>

      <TrayCard
        title="Org platform connector"
        kicker={draft.org_connector.enabled ? 'on' : 'off'}
      >
        <p
          style={{
            margin: '0 0 12px',
            color: 'var(--fg-muted)',
            fontSize: 12,
            lineHeight: 1.5,
          }}
        >
          Forward your in-game presence to a self-hosted org platform.
          Both sides must opt in — the org platform must also enable the
          link, and the token below is your member desktop token (same as
          the HUD).
        </p>

        <label
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            marginBottom: 12,
            fontSize: 13,
            cursor: 'pointer',
          }}
        >
          <input
            type="checkbox"
            aria-label="Enable org platform connector"
            checked={draft.org_connector.enabled}
            onChange={(e) =>
              updateOrgConnector({ enabled: e.target.checked })
            }
            style={{ accentColor: 'var(--accent)' }}
          />
          <span>Enable connector</span>
        </label>

        <Field
          label="Platform URL"
          hint="Must be https:// or wss:// (plaintext only for localhost)."
        >
          <TextInput
            type="text"
            value={draft.org_connector.platform_url ?? ''}
            placeholder="https://orgs.example"
            disabled={!draft.org_connector.enabled}
            aria-invalid={orgConnectorUrlError ? true : undefined}
            onChange={(e) =>
              updateOrgConnector({
                platform_url: e.target.value || null,
              })
            }
            spellCheck={false}
            autoComplete="off"
          />
          {orgConnectorUrlError && (
            <small
              role="alert"
              style={{ fontSize: 11, color: 'var(--danger)', lineHeight: 1.4 }}
            >
              {orgConnectorUrlError}
            </small>
          )}
        </Field>

        <Field
          label="Bearer token"
          hint="Stored in your OS keychain — never written to the config file, and never displayed after saving. Same as your member desktop token."
        >
          <p
            style={{
              margin: '0 0 8px',
              color: 'var(--fg-muted)',
              fontSize: 12,
              lineHeight: 1.5,
            }}
          >
            {bearerStatus === null ? (
              'Loading…'
            ) : bearerStatus.configured ? (
              <>
                Configured (last 4 chars:{' '}
                <code
                  style={{
                    color: 'var(--accent)',
                    fontFamily: 'var(--font-mono)',
                  }}
                >
                  {bearerStatus.preview ?? '????'}
                </code>
                ). Paste a new value to rotate.
              </>
            ) : (
              'Not set. Paste your member desktop token below.'
            )}
          </p>

          <TextInput
            type="password"
            value={bearerDraft}
            placeholder="•••••••••••••••••••••••••••"
            disabled={!draft.org_connector.enabled}
            aria-invalid={orgConnectorTokenError ? true : undefined}
            onChange={(e) => {
              setBearerDraft(e.target.value);
              setBearerSavedAt(null);
              setBearerError(null);
            }}
            spellCheck={false}
            autoComplete="off"
            aria-label="Org connector bearer token"
          />

          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              marginTop: 10,
            }}
          >
            <PrimaryButton
              type="button"
              onClick={handleSaveBearer}
              disabled={
                !draft.org_connector.enabled ||
                bearerSaving ||
                !bearerDraft.trim()
              }
            >
              {bearerSaving ? 'Saving…' : 'Save token'}
            </PrimaryButton>
            <GhostButton
              type="button"
              onClick={handleClearBearer}
              disabled={bearerSaving || !bearerStatus?.configured}
            >
              Clear
            </GhostButton>
            {bearerSavedAt && !bearerSaving && !bearerError && (
              <span style={{ fontSize: 11, color: 'var(--ok)' }}>✓ Saved</span>
            )}
          </div>

          {orgConnectorTokenError && (
            <small
              role="alert"
              style={{ fontSize: 11, color: 'var(--danger)', lineHeight: 1.4 }}
            >
              {orgConnectorTokenError}
            </small>
          )}
          {bearerError && (
            <small
              role="alert"
              style={{
                display: 'block',
                marginTop: 6,
                fontSize: 12,
                color: 'var(--danger)',
              }}
            >
              {bearerError}
            </small>
          )}
        </Field>
      </TrayCard>

      {pendingRemote && (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            padding: '10px 12px',
            background: 'var(--bg-elev)',
            border: '1px solid var(--accent)',
            borderRadius: 'var(--r-sm)',
            fontSize: 12,
          }}
          role="status"
        >
          <span style={{ flex: 1 }}>
            Cloud settings changed while you were editing.
          </span>
          <GhostButton
            type="button"
            onClick={() => {
              setDraft(pendingRemote);
              setSavedBaseline(pendingRemote);
              setPendingRemote(null);
            }}
          >
            Reload
          </GhostButton>
          <GhostButton
            type="button"
            onClick={() => setPendingRemote(null)}
          >
            Keep my changes
          </GhostButton>
        </div>
      )}

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          padding: '10px 0',
          borderTop: '1px solid var(--border)',
        }}
      >
        <PrimaryButton type="submit" disabled={saving}>
          {saving ? 'Saving…' : 'Save settings'}
        </PrimaryButton>
        <span style={{ fontSize: 11, color: 'var(--fg-dim)' }}>
          Changes apply on save. Sync state refetches automatically.
        </span>
        {savedAt && !saving && !error && (
          <span style={{ fontSize: 11, color: 'var(--ok)' }}>✓ Saved</span>
        )}
        {error && (
          <span style={{ fontSize: 11, color: 'var(--danger)' }}>{error}</span>
        )}
      </div>
    </form>
  );
}

/**
 * Single-line status string + optional download progress for the
 * Updates card. Kept as a separate component so the Updates JSX
 * stays readable; it doesn't need any of SettingsPane's state.
 */
function UpdateStatusLine({
  state,
  progress,
}: {
  state:
    | { kind: 'idle' }
    | { kind: 'checking' }
    | { kind: 'available'; info: UpdateInfo }
    | { kind: 'up_to_date' }
    | { kind: 'error'; message: string }
    | { kind: 'installing' };
  progress: { downloaded: number; total: number | null } | null;
}) {
  const baseStyle = {
    margin: 0,
    fontSize: 12,
    lineHeight: 1.5,
  } as const;
  switch (state.kind) {
    case 'idle':
      return (
        <p style={{ ...baseStyle, color: 'var(--fg-muted)' }}>
          Click "Check for updates" to query GitHub releases.
        </p>
      );
    case 'checking':
      return (
        <p style={{ ...baseStyle, color: 'var(--fg-muted)' }}>
          Checking for updates…
        </p>
      );
    case 'up_to_date':
      return (
        <p style={{ ...baseStyle, color: 'var(--ok)' }}>
          You're on the latest version.
        </p>
      );
    case 'available':
      return (
        <p style={{ ...baseStyle, color: 'var(--accent)' }}>
          Update available: <strong>v{state.info.version}</strong>
          {state.info.date && (
            <span style={{ color: 'var(--fg-dim)', fontSize: 11 }}>
              {' · '}
              {state.info.date}
            </span>
          )}
        </p>
      );
    case 'installing': {
      const pct =
        progress && progress.total && progress.total > 0
          ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
          : null;
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <p style={{ ...baseStyle, color: 'var(--accent)' }}>
            {pct !== null
              ? `Downloading… ${pct}%`
              : 'Downloading…'}
          </p>
          {progress && (
            <progress
              max={progress.total ?? undefined}
              value={progress.downloaded}
              style={{ width: '100%', height: 6 }}
            />
          )}
        </div>
      );
    }
    case 'error':
      return (
        <p style={{ ...baseStyle, color: 'var(--danger)' }}>
          {state.message}
        </p>
      );
  }
}

/**
 * Human-readable duration for the upload-queue ETA. Deliberately
 * coarse — the estimate is a rough "no worse than", so rendering it to
 * the second would imply precision it does not have.
 *
 * Exported for unit test; not used anywhere else.
 */
export function formatEta(secs: number): string {
  if (secs < 60) return 'under a minute';
  const mins = Math.round(secs / 60);
  if (mins < 60) return `about ${mins} min`;
  const hours = secs / 3600;
  if (hours < 24) return `about ${Math.round(hours)} h`;
  const days = Math.round(hours / 24);
  return `about ${days} day${days === 1 ? '' : 's'}`;
}

/** Thousands-separated count, so a six-figure backlog reads at a glance. */
function formatCount(n: number): string {
  return n.toLocaleString();
}

interface UploadQueueSectionProps {
  backlog: SyncBacklog | null;
  onUploadNow: () => void;
  uploading: boolean;
}

/**
 * Upload-queue readout: how many events are still on this machine, the
 * page size the next drain will use, and roughly how long clearing it
 * will take.
 *
 * Exists because the failure it describes is invisible otherwise: a
 * backlog drains silently in the background, and with the old
 * one-page-per-interval cadence a six-figure queue could sit for days
 * with every status indicator green. The mode line ("catching up" /
 * "paced — game running") is the part that answers "why is this slow?".
 */
function UploadQueueSection({
  backlog,
  onUploadNow,
  uploading,
}: UploadQueueSectionProps) {
  if (!backlog) return null;

  const { pending, catching_up, game_running, effective_batch_size, eta_secs } =
    backlog;
  const idle = pending === 0;

  let mode: string;
  if (idle) {
    mode = 'Queue empty — everything on this machine is uploaded.';
  } else if (catching_up && game_running) {
    mode =
      `Catching up at a reduced rate while Star Citizen is running, ` +
      `so the uplink does not compete with your session. ` +
      `${formatCount(effective_batch_size)} events per batch.`;
  } else if (catching_up) {
    mode =
      `Catching up — uploading back-to-back at ` +
      `${formatCount(effective_batch_size)} events per batch.`;
  } else {
    mode = `Uploading on the normal schedule, ${formatCount(
      effective_batch_size,
    )} events per batch.`;
  }

  return (
    <div
      data-testid="upload-queue"
      style={{
        border: '1px solid var(--border)',
        borderRadius: 6,
        padding: '8px 10px',
        margin: '0 0 10px',
        display: 'flex',
        flexDirection: 'column',
        gap: 6,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          justifyContent: 'space-between',
          gap: 8,
        }}
      >
        <span
          style={{
            fontSize: 11,
            fontWeight: 600,
            letterSpacing: 0.4,
            textTransform: 'uppercase',
            color: 'var(--fg-dim)',
          }}
        >
          Upload queue
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 14,
            fontWeight: 600,
            color: idle ? 'var(--ok)' : 'var(--fg)',
          }}
        >
          {formatCount(pending)}
        </span>
      </div>
      <small style={{ fontSize: 11, color: 'var(--fg-dim)', lineHeight: 1.4 }}>
        {mode}
        {!idle && eta_secs !== null && ` Estimated ${formatEta(eta_secs)}.`}
      </small>
      {!idle && (
        <div>
          <GhostButton type="button" onClick={onUploadNow} disabled={uploading}>
            {uploading ? 'Uploading…' : 'Upload now'}
          </GhostButton>
        </div>
      )}
    </div>
  );
}

interface DriftSectionProps {
  drift: UploadDrift | null;
  checking: boolean;
  requeueing: boolean;
  error: string | null;
  onCheck: () => void;
  onRequeue: () => void;
}

/**
 * On-demand local-vs-remote comparison, and the recovery it enables.
 *
 * This exists because the upload queue reading zero is not proof the server
 * has your data. The tray marks a row delivered on a 2xx and never looks
 * again, so if the server later loses events the queue stays empty and the
 * events sit unreachable in local storage. Nothing else surfaces that.
 *
 * Deliberately manual: drift changes when a server incident happens, not
 * continuously, so polling it would cost both sides constantly to answer a
 * question that is almost always "none".
 */
function DriftSection({
  drift,
  checking,
  requeueing,
  error,
  onCheck,
  onRequeue,
}: DriftSectionProps) {
  // The verdict is the TOTAL shortfall, never the summed per-type gaps.
  // Local reparse renames types in place while the server keeps the name
  // from upload time, so per-type differences appear on a perfectly healthy
  // pair and resending cannot close them.
  const short = drift ? drift.shortfall_total > 0 : false;
  const reclassified = drift?.rows.filter((r) => r.missing > 0) ?? [];
  const recoverable = short ? reclassified : [];

  return (
    <div
      data-testid="upload-drift"
      style={{
        border: '1px solid var(--border)',
        borderRadius: 6,
        padding: '8px 10px',
        margin: '0 0 10px',
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 8,
        }}
      >
        <span
          style={{
            fontSize: 11,
            fontWeight: 600,
            letterSpacing: 0.4,
            textTransform: 'uppercase',
            color: 'var(--fg-dim)',
          }}
        >
          Compare with server
        </span>
        <GhostButton type="button" onClick={onCheck} disabled={checking}>
          {checking ? 'Checking…' : 'Check now'}
        </GhostButton>
      </div>

      <small style={{ fontSize: 11, color: 'var(--fg-dim)', lineHeight: 1.4 }}>
        An empty upload queue means this app thinks everything was delivered —
        not that the server still has it. This compares the two.
      </small>

      {error && (
        <small style={{ fontSize: 11, color: 'var(--danger)' }}>{error}</small>
      )}

      {drift && (
        <>
          <div
            style={{
              display: 'flex',
              gap: 16,
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
            }}
          >
            <span>
              <span style={{ color: 'var(--fg-dim)' }}>here </span>
              {drift.local_sent_total.toLocaleString()}
            </span>
            <span>
              <span style={{ color: 'var(--fg-dim)' }}>server </span>
              {drift.remote_total.toLocaleString()}
            </span>
            {drift.pending > 0 && (
              <span style={{ color: 'var(--fg-dim)' }}>
                queued {drift.pending.toLocaleString()}
              </span>
            )}
          </div>

          {!short ? (
            <>
              <small style={{ fontSize: 11, color: 'var(--ok)' }}>
                ✓ The server has everything this device uploaded
                {drift.surplus_total > 0 &&
                  `, plus ${drift.surplus_total.toLocaleString()} more`}
                .
              </small>
              {reclassified.length > 0 && (
                <small
                  style={{ fontSize: 11, color: 'var(--fg-dim)', lineHeight: 1.4 }}
                >
                  {reclassified.length}{' '}
                  {reclassified.length === 1 ? 'type differs' : 'types differ'} by
                  name only. The parser renamed them on this device after they
                  were uploaded, so the server still lists them under the older
                  name. Nothing is missing and re-sending would change nothing.
                </small>
              )}
            </>
          ) : (
            <>
              <small
                style={{
                  fontSize: 12,
                  color: 'var(--warn)',
                  lineHeight: 1.4,
                }}
              >
                The server is short {drift.shortfall_total.toLocaleString()}{' '}
                events. The types below are the likeliest candidates; they are
                still on this machine and can be sent again.
              </small>

              <div
                style={{
                  maxHeight: 180,
                  overflowY: 'auto',
                  fontFamily: 'var(--font-mono)',
                  fontSize: 11,
                }}
              >
                <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                  <thead>
                    <tr style={{ color: 'var(--fg-dim)', textAlign: 'left' }}>
                      <th style={{ fontWeight: 400, padding: '2px 0' }}>type</th>
                      <th style={{ fontWeight: 400, textAlign: 'right' }}>here</th>
                      <th style={{ fontWeight: 400, textAlign: 'right' }}>server</th>
                      <th style={{ fontWeight: 400, textAlign: 'right' }}>missing</th>
                    </tr>
                  </thead>
                  <tbody>
                    {recoverable.map((r) => (
                      <tr key={r.event_type}>
                        <td style={{ padding: '2px 0' }}>{r.event_type}</td>
                        <td style={{ textAlign: 'right' }}>
                          {r.local_sent.toLocaleString()}
                        </td>
                        <td style={{ textAlign: 'right' }}>
                          {r.remote.toLocaleString()}
                        </td>
                        <td
                          style={{ textAlign: 'right', color: 'var(--warn)' }}
                        >
                          {r.missing.toLocaleString()}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>

              <div>
                <PrimaryButton
                  type="button"
                  onClick={onRequeue}
                  disabled={requeueing}
                >
                  {requeueing
                    ? 'Queueing…'
                    : `Send ${drift.shortfall_total.toLocaleString()} again`}
                </PrimaryButton>
              </div>
              <small
                style={{ fontSize: 11, color: 'var(--fg-dim)', lineHeight: 1.4 }}
              >
                Safe to run more than once — the server ignores anything it
                already has, so nothing gets duplicated.
              </small>
            </>
          )}


        </>
      )}
    </div>
  );
}

interface SyncSpeedSectionProps {
  draft: Config;
  updateRemote: (patch: Partial<Config['remote_sync']>) => void;
}

/**
 * Radio-card preset picker for sync cadence. The Rust side has a
 * `set_sync_preset` command but we drive everything through the
 * existing `updateRemote` flow so the draft stays the single source
 * of truth — Save commits intervals + the rest in one round-trip.
 *
 * The active preset is derived from the current interval pair via
 * `detectSyncPreset`; a config that doesn't match any named pair
 * resolves to 'custom' which reveals the raw number inputs (which
 * are always rendered below anyway — selecting Custom is more of a
 * visual cue than a mode switch).
 */
function SyncSpeedSection({ draft, updateRemote }: SyncSpeedSectionProps) {
  const active: SyncPreset = detectSyncPreset(draft.remote_sync);

  const onPick = (preset: SyncPreset) => {
    if (preset === 'custom') {
      // Custom is a UI marker — keep current intervals as the user
      // commits to whatever numbers they type below.
      return;
    }
    const choice = SYNC_PRESETS.find((p) => p.id === preset);
    if (!choice) return;
    updateRemote({
      priority_interval_secs: choice.priorityInterval,
      interval_secs: choice.bulkInterval,
    });
  };

  return (
    <fieldset
      style={{
        border: '1px solid var(--border)',
        borderRadius: 6,
        padding: '8px 10px',
        margin: '0 0 10px',
      }}
    >
      <legend
        style={{
          padding: '0 6px',
          fontSize: 11,
          fontWeight: 600,
          letterSpacing: 0.4,
          textTransform: 'uppercase',
          color: 'var(--fg-dim)',
        }}
      >
        Sync speed
      </legend>
      <div
        role="radiogroup"
        aria-label="Sync speed preset"
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(4, 1fr)',
          gap: 6,
        }}
      >
        {SYNC_PRESETS.map((p) => {
          const selected = p.id === active;
          return (
            <button
              key={p.id}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => onPick(p.id)}
              style={{
                background: selected ? 'var(--accent-soft)' : 'transparent',
                border: `1px solid ${
                  selected ? 'var(--accent)' : 'var(--border)'
                }`,
                borderRadius: 6,
                padding: '8px 6px',
                cursor: 'pointer',
                textAlign: 'left',
                color: 'var(--fg)',
                display: 'flex',
                flexDirection: 'column',
                gap: 2,
              }}
            >
              <span style={{ fontSize: 12, fontWeight: 600 }}>{p.label}</span>
              <span style={{ fontSize: 10, color: 'var(--fg-dim)' }}>
                {p.id === 'custom'
                  ? 'Pick your own intervals'
                  : `${p.priorityInterval}s / ${p.bulkInterval}s`}
              </span>
            </button>
          );
        })}
      </div>
      <p
        style={{
          margin: '8px 0 0',
          fontSize: 11,
          color: 'var(--fg-dim)',
          lineHeight: 1.4,
        }}
      >
        Priority lane drains{' '}
        <strong style={{ color: 'var(--fg)' }}>
          location, deaths, vehicle destruction, quantum target, session end
        </strong>{' '}
        on the fast schedule. Everything else waits for the bulk schedule.
      </p>
    </fieldset>
  );
}
