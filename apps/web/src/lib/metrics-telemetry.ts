/**
 * Frontend telemetry for the metrics surfaces.
 *
 * Per the impl plan §2.4 + §5 telemetry gate, every chart card fires
 * `recordMetricView` on mount so we can answer "did anyone look at
 * this surface?" before committing to building more like it.
 *
 * Fire-and-forget: a single POST to `/v1/me/telemetry`. Errors are
 * swallowed (best-effort) — telemetry must never break the page.
 */

'use client';

export interface MetricViewPayload {
  surface: string;
  mode?: string;
}

function hasConsent(): boolean {
  // Re-read each call — caching at module scope poisons the value
  // permanently when Next's RSC pass first evaluates this module on
  // the server (where `window` is undefined). The localStorage read
  // is cheap; no need to memoise.
  if (typeof window === 'undefined') return false;
  return window.localStorage.getItem('starstats.telemetry') === 'true';
}

export function recordMetricView(payload: MetricViewPayload): void {
  if (!hasConsent()) return;
  if (typeof window === 'undefined') return;
  const body = JSON.stringify({
    ...payload,
    occurred_at: new Date().toISOString(),
  });
  try {
    const url = '/v1/me/telemetry';
    if (navigator.sendBeacon) {
      const blob = new Blob([body], { type: 'application/json' });
      navigator.sendBeacon(url, blob);
      return;
    }
    void fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body,
      keepalive: true,
    }).catch(() => {});
  } catch {
    // Telemetry must never break the page.
  }
}
