/**
 * Per-hour activity heatmap. Buckets every event in the trace window
 * (typically 7d) by hour-of-day (0–23, local timezone) and renders a
 * single-row heatmap so the user can answer "when do I tend to play?"
 * at a glance.
 *
 * Single-row layout chosen over a 7-day grid because a 24×7 matrix
 * needs well-distributed activity across the week to read; the flat
 * 24-bucket row degrades gracefully — even one event lights a cell.
 *
 * Server component, pure props. Hour bucketing uses the BROWSER's
 * timezone via `Date#getHours()`, matching the rest of the journey
 * components' wall-clock convention.
 */

import React from 'react';
import type { TraceEntry } from '@/lib/api';

interface Props {
  entries: TraceEntry[];
  /** Window the parent fetched. Used only for the caption. */
  windowHours: number;
}

export function LocationActivityHeatmap({ entries, windowHours }: Props) {
  const buckets = bucketByHour(entries);
  const total = buckets.reduce((s, n) => s + n, 0);
  const max = Math.max(...buckets, 1);
  const peakHour = total === 0 ? null : buckets.indexOf(max);

  if (total === 0) {
    return (
      <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
        Not enough activity in the window to chart your play hours yet.
      </p>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div
        role="img"
        aria-label={`Activity by hour of day over the last ${formatWindow(windowHours)}`}
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(24, 1fr)',
          gap: 2,
        }}
      >
        {buckets.map((count, hr) => {
          const intensity = count / max;
          return (
            <div
              key={hr}
              title={`${formatHour(hr)} — ${count} event${count === 1 ? '' : 's'}`}
              style={{
                height: 28,
                borderRadius: 2,
                background: cellColor(intensity),
                border:
                  hr === peakHour
                    ? '1px solid var(--accent)'
                    : '1px solid transparent',
              }}
            />
          );
        })}
      </div>
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(24, 1fr)',
          gap: 2,
          fontSize: 9,
          color: 'var(--fg-dim)',
          fontFamily: 'var(--font-mono, ui-monospace, monospace)',
        }}
      >
        {Array.from({ length: 24 }, (_, hr) => (
          <span
            key={hr}
            style={{ textAlign: 'center', opacity: hr % 3 === 0 ? 1 : 0 }}
          >
            {hr.toString().padStart(2, '0')}
          </span>
        ))}
      </div>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontSize: 11,
          color: 'var(--fg-dim)',
        }}
      >
        <span>
          {peakHour !== null && (
            <>
              Peak around{' '}
              <strong style={{ color: 'var(--fg)' }}>
                {formatHour(peakHour)}
              </strong>
              <span style={{ marginLeft: 6 }}>
                ({buckets[peakHour].toLocaleString()} event
                {buckets[peakHour] === 1 ? '' : 's'})
              </span>
            </>
          )}
        </span>
        <span>
          {total.toLocaleString()} events · {formatWindow(windowHours)}
        </span>
      </div>
    </div>
  );
}

function bucketByHour(entries: TraceEntry[]): number[] {
  const out = new Array<number>(24).fill(0);
  for (const e of entries) {
    const d = new Date(e.started_at);
    if (Number.isNaN(d.getTime())) continue;
    out[d.getHours()] += e.event_count;
  }
  return out;
}

function cellColor(intensity: number): string {
  if (intensity <= 0) return 'var(--bg-elev)';
  const pct = Math.round(intensity * 100);
  return `color-mix(in oklab, var(--accent) ${pct}%, var(--bg-elev))`;
}

function formatHour(hr: number): string {
  return `${hr.toString().padStart(2, '0')}:00`;
}

function formatWindow(hours: number): string {
  if (hours <= 24) return `${hours}h`;
  const days = Math.round(hours / 24);
  return `${days}d`;
}
