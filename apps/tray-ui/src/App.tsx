import { useCallback, useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { api, type Config, type SettingsField } from './api';
import { StatusPane } from './components/StatusPane';
import { SettingsPane } from './components/SettingsPane';
import { LogsPane } from './components/LogsPane';
import { KbPane } from './components/KbPane';
import { WhatsNewPane } from './panes/WhatsNewPane';
import { TrayHeader, type TrayView } from './components/TrayHeader';
import { SubmissionsPane } from './submissions/SubmissionsPane';
import { useStatusPolling } from './hooks/useStatusPolling';
import { FieldFocusProvider, useFieldFocus } from './hooks/useFieldFocus';
import {
  loadAllReferenceBundles,
  type AllReferenceBundles,
  EMPTY_ALL_BUNDLES,
} from './lib/reference';
import type { PrettyLookup } from './components/tray/format';
import { DEFAULT_WAVE_SPEED, isWaveSpeed } from './lib/wave-speed';
import './styles.css';

/** How often the unknown-line badge polls storage. Cheap query
 *  (single indexed COUNT) so 30s is comfortable; tweak down if a
 *  flood of unknowns mid-session feels stale. */
const UNKNOWN_BADGE_REFRESH_MS = 30_000;

function AppInner() {
  const [view, setView] = useState<TrayView>('status');
  const [config, setConfig] = useState<Config | null>(null);
  const [error, setError] = useState<string | null>(null);
  const fieldFocus = useFieldFocus();

  // Apply the persisted theme to the document root. `index.html` ships
  // with `data-theme="stanton"` so the unstyled-flash before config
  // loads is still a valid theme; once config arrives we swap the
  // attribute and the four `[data-theme="..."]` token blocks in
  // `starstats-tokens.css` repaint without a reflow.
  //
  // Deliberately INSTANT — no `applyThemeWithWave` here. The wave is a
  // user-initiated-switch affordance (see `SettingsPane.updateTheme`);
  // replaying it on every app boot / config refresh would be visual
  // noise the user never asked for.
  useEffect(() => {
    if (config?.theme) {
      document.documentElement.dataset.theme = config.theme;
    }
  }, [config?.theme]);

  // Stamp the wave-speed preference onto the document root so
  // `lib/theme-transition.ts`'s `resolveWaveSpeedMs()` reads it the
  // same way the web client reads `data-wave-speed` off `<html>` (no
  // SSR here, so this boot-time effect is the tray's stand-in for the
  // server-rendered attribute). Falls back to the default when config
  // hasn't loaded yet or carries an unrecognised value.
  useEffect(() => {
    const speed = config?.theme_wave_speed;
    document.documentElement.dataset.waveSpeed = isWaveSpeed(speed)
      ? speed
      : DEFAULT_WAVE_SPEED;
  }, [config?.theme_wave_speed]);
  // Sourced via the Tauri command (Rust CARGO_PKG_VERSION) rather
  // than a Vite build-time constant from `package.json`, because
  // `package.json` was the wrong source of truth — it shipped at
  // 0.1.0 while the workspace Cargo.toml advanced through several
  // releases. One source of truth (the Rust binary), one fetch.
  const [appVersion, setAppVersion] = useState<string | null>(null);

  // Poll while either Status or Settings is foregrounded. Settings'
  // Remote sync card surfaces a live health pill (OK / ERR / IDLE /
  // OFF) derived from `SyncStats`, so it needs the same fresh snapshot
  // the Status view already consumes. Other views (logs, review) skip
  // polling — they don't render any sync-derived UI.
  const { status, refresh: refreshStatus } = useStatusPolling({
    active: view === 'status' || view === 'settings',
    onError: setError,
  });

  // Local count of unknown shapes pending review. Drives the badge on
  // the Review tab; SubmissionsPane reports back via `onCountChange`
  // when it refetches after a Submit/Dismiss so we don't double-poll.
  const [unknownCount, setUnknownCount] = useState(0);
  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const n = await api.countUnknownLines();
        if (!cancelled) setUnknownCount(n);
      } catch {
        // Badge is informational — silent fallback keeps the
        // header noise-free if the IPC layer hiccups.
      }
    };
    void tick();
    const id = window.setInterval(tick, UNKNOWN_BADGE_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const refreshConfig = useCallback(async () => {
    try {
      const c = await api.getConfig();
      setConfig(c);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refreshConfig();
    let cancelled = false;
    api
      .getAppVersion()
      .then((v) => {
        if (!cancelled) setAppVersion(v);
      })
      .catch(() => {
        // Version is informational; if it fails the header just
        // omits it. Already shown in Settings → Updates with its
        // own error path, so silent fallback is fine here.
      });
    return () => {
      cancelled = true;
    };
  }, [refreshConfig]);

  // Reference catalogues — drives class-name prettification in
  // the timeline (Readout + Manifest panes) and the KB browse pane.
  // Refetched whenever the paired API URL changes; rate-limit is
  // generous (10/s burst 40) and the four endpoints are cached an
  // hour server-side, so cold-loading once per session is cheap.
  const [bundles, setBundles] = useState<AllReferenceBundles>(EMPTY_ALL_BUNDLES);
  const apiUrl = config?.remote_sync.api_url ?? null;
  useEffect(() => {
    if (!apiUrl) {
      setBundles(EMPTY_ALL_BUNDLES);
      return;
    }
    let cancelled = false;
    loadAllReferenceBundles(apiUrl)
      .then((b) => {
        if (!cancelled) setBundles(b);
      })
      .catch(() => {
        // Catalogue is cosmetic — keep the timeline working even
        // if the server's reference endpoints are unreachable.
      });
    return () => {
      cancelled = true;
    };
  }, [apiUrl]);

  // Build a flat lookup once per bundle refresh. The timeline surfaces
  // consume this to rewrite class-name tokens inline. Memoised on `bundles`
  // so its identity is STABLE across unrelated re-renders (M-U6): LogsPane
  // keys its per-row title memo on this, so a fresh Map every render would
  // defeat that memo and re-run the O(catalog × rows) scan on every tick.
  const prettyLookup: PrettyLookup = useMemo(() => {
    const m = new Map<string, string>();
    for (const cat of ['vehicle', 'weapon', 'item', 'location'] as const) {
      for (const e of bundles[cat].list) {
        m.set(e.class_name.toLowerCase(), e.display_name);
      }
    }
    return m;
  }, [bundles]);

  // Remote-sync config downloads: the bulk-lane piggyback in
  // `crates/starstats-client/src/sync.rs` emits `config-changed` after
  // persisting a server-newer snapshot. Listening at the App level
  // (not inside SettingsPane) is load-bearing — Settings is one of
  // four tabs, and an event fired while the user is on Status/Logs/
  // Review would otherwise have no subscriber and be lost.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<Config>('config-changed', (e) => {
      setConfig(e.payload);
    }).then((unl) => {
      // Unmounted before listen() resolved → detach immediately so the
      // listener doesn't leak (M-U5).
      if (cancelled) unl();
      else unlisten = unl;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const onSaveConfig = async (next: Config) => {
    await api.saveConfig(next);
    setConfig(next);
    void refreshStatus();
  };

  // HealthCard CTAs land here: switch the view to Settings and focus
  // the targeted field via the ref registry. SettingsPane registers
  // its field refs in an effect; useFieldFocus.focus retries on
  // animation frames until the ref shows up (bounded).
  const onGoToSettings = useCallback(
    (field: SettingsField) => {
      setView('settings');
      fieldFocus.focus(field);
    },
    [fieldFocus]
  );

  const isTailing = status?.tail.current_path != null;

  return (
    <div className="app">
      <TrayHeader
        view={view}
        onView={setView}
        isTailing={isTailing}
        version={appVersion}
        reviewBadge={unknownCount}
      />
      <main className="app__main">
        {error && <div className="error">Error: {error}</div>}
        {/*
          Keyed wrapper triggers the design system's `.ss-screen-enter`
          fade-and-lift animation every time the user switches tabs;
          inside, the staggered `.ss-card` mount-in animations cascade
          per the nth-child rules in `starstats-tokens.css`.
        */}
        <div key={view} className="ss-screen-enter">
          {view === 'status' &&
            (status ? (
              <StatusPane
                status={status}
                webOrigin={config?.web_origin ?? null}
                onGoToSettings={onGoToSettings}
                prettyLookup={prettyLookup}
                bundles={bundles}
                channel={config?.release_channel ?? 'live'}
                autoCheck={config?.auto_update_check ?? false}
              />
            ) : (
              <div className="loading">Loading…</div>
            ))}
          {view === 'logs' && (
            <LogsPane
              prettyLookup={prettyLookup}
              bundles={bundles}
              webOrigin={config?.web_origin ?? null}
            />
          )}
          {view === 'kb' && (
            <KbPane
              apiUrl={config?.remote_sync.api_url ?? null}
              webOrigin={config?.web_origin ?? null}
            />
          )}
          {view === 'whats-new' && (
            <WhatsNewPane webOrigin={config?.web_origin ?? null} />
          )}
          {view === 'review' && (
            <SubmissionsPane
              onCountChange={setUnknownCount}
              handle={config?.remote_sync.claimed_handle ?? null}
            />
          )}
          {view === 'settings' &&
            (config ? (
              <SettingsPane
                config={config}
                onSave={onSaveConfig}
                status={status}
              />
            ) : (
              <div className="loading">Loading…</div>
            ))}
        </div>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <FieldFocusProvider>
      <AppInner />
    </FieldFocusProvider>
  );
}
