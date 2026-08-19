'use client';

import { useCallback, useEffect, useState } from 'react';
import type { Units } from './kb-viz';
import { saveKbPrefs } from '@/app/kb/actions';

export type KbView = 'visual' | 'compact';
export const KB_PREFS_STORAGE_KEY = 'ss-kb-prefs';

export interface KbPrefs {
  view: KbView;
  units: Units;
}

const DEFAULTS: KbPrefs = { view: 'visual', units: 'metric' };

function readLocal(): Partial<KbPrefs> | null {
  try {
    const raw = localStorage.getItem(KB_PREFS_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<KbPrefs>;
    return parsed && typeof parsed === 'object' ? parsed : null;
  } catch {
    return null;
  }
}

/** Drop keys whose value is `undefined` so they don't overwrite earlier
 * spread layers. Server prefs arrive as `{ kb_view, kb_units }` mapped to
 * `{ view, units }` where either field may be `undefined`. */
function defined<T extends object>(o: T | null | undefined): Partial<T> {
  if (!o) return {};
  return Object.fromEntries(
    Object.entries(o).filter(([, v]) => v !== undefined),
  ) as Partial<T>;
}

/**
 * Resolve the initial prefs with per-field precedence: server > localStorage
 * > defaults. `undefined` values are ignored at each layer so a partial
 * server pref (e.g. only `view`) keeps the user's local `units`. Reads
 * localStorage so it's unit-testable.
 */
export function resolveInitialKbPrefs(serverPrefs: Partial<KbPrefs> | null): KbPrefs {
  return { ...DEFAULTS, ...defined(readLocal()), ...defined(serverPrefs) };
}

/**
 * KB view-config state. `signedIn` decides persistence target: profile
 * (server action) for signed-in users, localStorage for anonymous. The
 * localStorage mirror is always written so the choice is instant + sticky
 * across the session even if the server PUT fails.
 */
export function useKbPrefs(opts: {
  serverPrefs: Partial<KbPrefs> | null;
  signedIn: boolean;
}) {
  // SSR-safe init: server-or-defaults only, NO localStorage read. Both the
  // server render and the first client render produce the same value, so
  // hydration can't mismatch (a saved local pref would otherwise diverge).
  const [prefs, setPrefs] = useState<KbPrefs>(() => ({
    ...DEFAULTS,
    ...defined(opts.serverPrefs),
  }));

  // After mount (client-only), reconcile with localStorage. This is where
  // server > local > default precedence applies — same logic as
  // resolveInitialKbPrefs. serverPrefs is stable per page render.
  useEffect(() => {
    setPrefs(resolveInitialKbPrefs(opts.serverPrefs));
    // run once on mount; serverPrefs is stable per page render
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const update = useCallback(
    (patch: Partial<KbPrefs>) => {
      setPrefs((prev) => {
        const next = { ...prev, ...patch };
        try {
          localStorage.setItem(KB_PREFS_STORAGE_KEY, JSON.stringify(next));
        } catch {
          /* private mode / quota — non-fatal */
        }
        if (opts.signedIn) {
          // Map to the wire field names (kb_view / kb_units).
          void saveKbPrefs({ kb_view: next.view, kb_units: next.units });
        }
        return next;
      });
    },
    [opts.signedIn],
  );

  return { prefs, update };
}
