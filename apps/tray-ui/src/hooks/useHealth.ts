import { useEffect, useState } from 'react';
import { api, type HealthItem } from '../api';

const POLL_MS = 15_000;

/**
 * Polls `get_health` on a 15s cadence (matching the existing status
 * poll). Returns the most-recent successful list plus a refresh
 * function. Failures are silenced — the Health UI is non-blocking,
 * and per-call surfacing would be noisy. The next successful poll
 * supersedes the silent failure.
 */
export function useHealth(): { items: HealthItem[]; refresh: () => Promise<void> } {
  const [items, setItems] = useState<HealthItem[]>([]);

  const refresh = async () => {
    try {
      const next = await api.getHealth();
      setItems(next);
    } catch {
      // intentionally swallowed; see fn doc
    }
  };

  useEffect(() => {
    let cancelled = false;
    let handle: number | undefined;

    const start = () => {
      if (handle !== undefined) return;
      void refresh();
      handle = window.setInterval(() => {
        if (!cancelled) void refresh();
      }, POLL_MS);
    };
    const stop = () => {
      if (handle !== undefined) {
        window.clearInterval(handle);
        handle = undefined;
      }
    };

    const onVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        // Returning from hidden → immediate refresh, then resume polling.
        start();
      } else {
        stop();
      }
    };

    if (document.visibilityState === 'visible') start();
    document.addEventListener('visibilitychange', onVisibilityChange);

    return () => {
      cancelled = true;
      stop();
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
  }, []);

  return { items, refresh };
}
