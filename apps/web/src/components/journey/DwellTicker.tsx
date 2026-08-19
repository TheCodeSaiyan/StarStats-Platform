'use client';

/**
 * Live "here for X" counter. Re-renders once per second so the dwell
 * tail in the journey location hero feels alive instead of frozen at
 * page-load time.
 *
 * Client component, pure — takes the `enteredAt` ISO timestamp as a
 * prop. Counts up from that moment using the wall clock; if the
 * server's clock and the browser's clock drift, the displayed delta
 * is the browser's view, which is fine for a "how long have I been
 * here?" affordance (the user can't act on it anyway).
 */

import { useEffect, useState } from 'react';

interface Props {
  /** ISO 8601 timestamp of when the user entered the current stop. */
  enteredAt: string;
}

export function DwellTicker({ enteredAt }: Props) {
  const startMs = parseStart(enteredAt);
  const [nowMs, setNowMs] = useState<number>(() =>
    typeof window === 'undefined' ? startMs ?? 0 : Date.now(),
  );

  useEffect(() => {
    if (startMs === null) return;
    const id = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [startMs]);

  if (startMs === null) {
    return <span className="mono">—</span>;
  }

  const seconds = Math.max(0, Math.floor((nowMs - startMs) / 1000));
  return <span className="mono">{formatDwellLive(seconds)}</span>;
}

function parseStart(iso: string): number | null {
  const t = new Date(iso).getTime();
  return Number.isFinite(t) ? t : null;
}

/**
 * Like `formatDwell` from trail-utils but renders seconds explicitly
 * when the dwell is under a minute so the ticker visibly counts up.
 */
function formatDwellLive(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  if (mins < 60) return `${mins}m ${secs.toString().padStart(2, '0')}s`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  return remMins === 0
    ? `${hours}h`
    : `${hours}h ${remMins.toString().padStart(2, '0')}m`;
}
