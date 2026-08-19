/**
 * MetricCard — the mandatory shell every metrics surface mounts into.
 *
 * Enforces the cross-cutting checklist from impl plan §2 via the
 * TypeScript type system:
 *   - `flagKey`        feature-flag gate (§2.4)
 *   - `telemetryKey`   recordMetricView identifier (§2.4)
 *   - `empty`          empty-state rendering (§2.3)
 *   - `error`          error-state rendering (§2.3)
 *   - `srTable`        screen-reader fallback (§2.1 a11y)
 *
 * Required props (no defaults) mean a card that skips any of these
 * fails `tsc` rather than silently degrading.
 */

'use client';

import { useEffect, type ReactNode } from 'react';
import { isFlagEnabled, type MetricFlagKey } from '@/lib/feature-flags';
import { recordMetricView } from '@/lib/metrics-telemetry';

export interface MetricCardProps {
  title: string;
  caption?: string;
  flagKey: MetricFlagKey;
  telemetryKey: string;
  empty: ReactNode;
  error: ReactNode;
  srTable: ReactNode;
  bestEffort?: { reason: string };
  mode: 'data' | 'empty' | 'error';
  children: ReactNode;
}

export function MetricCard(props: MetricCardProps) {
  const {
    title,
    caption,
    flagKey,
    telemetryKey,
    empty,
    error,
    srTable,
    bestEffort,
    mode,
    children,
  } = props;

  const enabled = isFlagEnabled(flagKey);

  // Telemetry only fires for enabled, data-mode views — disabled
  // surfaces shouldn't pollute "did anyone look at this?" analytics
  // (reviewer-flagged: ghost telemetry for hidden cards).
  useEffect(() => {
    if (!enabled) return;
    if (mode !== 'data') return;
    recordMetricView({ surface: telemetryKey, mode });
  }, [enabled, telemetryKey, mode]);

  if (!enabled) return null;

  return (
    <section className="ss-card metric-card" aria-labelledby={`metric-${telemetryKey}-title`}>
      <header className="metric-card__header">
        <div>
          <h2 id={`metric-${telemetryKey}-title`} className="metric-card__title">
            {title}
          </h2>
          {caption ? <p className="metric-card__caption">{caption}</p> : null}
        </div>
        {bestEffort ? (
          <span
            className="metric-card__badge metric-card__badge--best-effort"
            title={bestEffort.reason}
            aria-label={`Best-effort data: ${bestEffort.reason}`}
          >
            best-effort
          </span>
        ) : null}
      </header>
      {mode === 'error' ? (
        <div className="metric-card__error" role="alert">
          {error}
        </div>
      ) : mode === 'empty' ? (
        <div className="metric-card__empty">{empty}</div>
      ) : (
        <>
          <div className="metric-card__body">{children}</div>
          <div className="metric-card__sr-table sr-only">{srTable}</div>
        </>
      )}
    </section>
  );
}
