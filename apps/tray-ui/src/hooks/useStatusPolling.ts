import { useCallback, useEffect, useState } from 'react';
import { api, type StatusResponse } from '../api';

const POLL_MS = 2_000;

export interface StatusPollingOptions {
  /** Whether polling is active. Pass `view === 'status'`. */
  active: boolean;
  /**
   * Optional shared error setter. Each fetch — successful or not —
   * writes through this setter (success → `null`, failure →
   * `String(e)`), preserving the legacy "single error slot, last
   * writer wins" UX.
   */
  onError?: (msg: string | null) => void;
}

export interface StatusPollingResult {
  status: StatusResponse | null;
  refresh: () => Promise<void>;
}

/**
 * Polls `api.getStatus()` every 2s while `active` is true.
 *
 * Fires an immediate fetch on mount/activate to avoid a 2s gap before
 * the first paint. `refresh` performs a one-shot fetch independent of
 * `active` — used to refresh status after a config save (since polling
 * is paused while not on the status view).
 */
export function useStatusPolling(
  options: StatusPollingOptions,
): StatusPollingResult {
  const { active, onError } = options;
  const [status, setStatus] = useState<StatusResponse | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await api.getStatus();
      setStatus(s);
      onError?.(null);
    } catch (e) {
      onError?.(String(e));
    }
  }, [onError]);

  useEffect(() => {
    if (!active) return;
    let alive = true;
    const tick = async () => {
      try {
        const s = await api.getStatus();
        if (alive) {
          setStatus(s);
          onError?.(null);
        }
      } catch (e) {
        if (alive) onError?.(String(e));
      }
    };
    void tick();
    const timer = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [active, onError]);

  return { status, refresh };
}
