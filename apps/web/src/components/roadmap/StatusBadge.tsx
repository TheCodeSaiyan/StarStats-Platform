/**
 * Small inline pill for a roadmap headline status or per-channel
 * status. Colours are tone-tinted (CSS vars from globals.css) — no
 * hard-coded hex.
 */
import React from 'react';

type Tone = 'neutral' | 'progress' | 'positive' | 'paused';

const TONE_FOR_STATUS: Record<string, Tone> = {
  proposed: 'neutral',
  'in-design': 'neutral',
  building: 'progress',
  beta: 'progress',
  shipped: 'positive',
  parked: 'paused',
};

const TONE_BG: Record<Tone, string> = {
  neutral: 'var(--bg-elev)',
  progress: 'var(--accent-dim, var(--bg-elev))',
  positive: 'var(--positive-dim, var(--bg-elev))',
  paused: 'var(--warn-dim, var(--bg-elev))',
};

const TONE_FG: Record<Tone, string> = {
  neutral: 'var(--fg-dim)',
  progress: 'var(--accent, var(--fg))',
  positive: 'var(--positive, var(--fg))',
  paused: 'var(--warn, var(--fg-dim))',
};

const LABEL_OVERRIDE: Record<string, string> = {
  'in-design': 'In design',
  'tech-preview': 'Tech preview',
};

function labelFor(status: string): string {
  if (LABEL_OVERRIDE[status]) return LABEL_OVERRIDE[status];
  return status.charAt(0).toUpperCase() + status.slice(1);
}

export function StatusBadge({ status }: { status: string }) {
  const tone = TONE_FOR_STATUS[status] ?? 'neutral';
  return (
    <span
      data-status={status}
      style={{
        display: 'inline-block',
        padding: '2px 8px',
        borderRadius: 999,
        fontSize: 11,
        fontWeight: 600,
        letterSpacing: 0.2,
        textTransform: 'none',
        background: TONE_BG[tone],
        color: TONE_FG[tone],
        border: '1px solid var(--border)',
      }}
    >
      {labelFor(status)}
    </span>
  );
}
