import React from 'react';
import type { SupporterStatusDto } from '@/lib/api';

/**
 * Supporter status chip. Renders the "Supporter" pill (plus optional
 * 28-char name plate) for users whose `supporter_status.state` is
 * `active` or `lapsed`. Returns `null` when state is `none` so the
 * caller can drop the component without a conditional render.
 *
 * Tier-specific styling (per user-chosen design): coffee = warm
 * brown; standard = accent; generous = gold. Lapsed states mute
 * those colours per the design promise — "the pill stays —
 * recognition is permanent — but accent perks revert to free-tier
 * until the next payment lands" (see `docs/REVOLUT-INTEGRATION-PLAN.md`).
 *
 * When `tier_key` is `null` (no completed order yet, defensive
 * fallback) the chip uses the standard accent palette so it still
 * renders.
 */

export type SupporterChipSize = 'sm' | 'md';

type TierKey = 'coffee' | 'standard' | 'generous';

interface TierPalette {
  fg: string;
  bg: string;
  border: string;
}

// Each tier resolves to a small palette built off CSS custom
// properties so the colours respect the active theme (Stanton / Pyro
// / future themes). The `color-mix` literals scale alpha against the
// base hue so callers don't have to think about contrast separately.
const TIER_PALETTES: Record<TierKey, { active: TierPalette; lapsed: TierPalette }> = {
  coffee: {
    active: {
      fg: 'var(--supporter-coffee-fg, #b87333)',
      bg: 'color-mix(in oklab, var(--supporter-coffee-fg, #b87333) 14%, transparent)',
      border: 'var(--supporter-coffee-fg, #b87333)',
    },
    lapsed: {
      fg: 'var(--fg-muted)',
      bg: 'color-mix(in oklab, var(--supporter-coffee-fg, #b87333) 6%, transparent)',
      border: 'var(--border)',
    },
  },
  standard: {
    active: {
      fg: 'var(--accent)',
      bg: 'color-mix(in oklab, var(--accent) 12%, transparent)',
      border: 'var(--accent)',
    },
    lapsed: {
      fg: 'var(--fg-muted)',
      bg: 'color-mix(in oklab, var(--accent) 6%, transparent)',
      border: 'var(--border)',
    },
  },
  generous: {
    active: {
      fg: 'var(--supporter-generous-fg, #d4af37)',
      bg: 'color-mix(in oklab, var(--supporter-generous-fg, #d4af37) 14%, transparent)',
      border: 'var(--supporter-generous-fg, #d4af37)',
    },
    lapsed: {
      fg: 'var(--fg-muted)',
      bg: 'color-mix(in oklab, var(--supporter-generous-fg, #d4af37) 6%, transparent)',
      border: 'var(--border)',
    },
  },
};

const TIER_LABELS: Record<TierKey, string> = {
  coffee: 'Coffee supporter',
  standard: 'Supporter',
  generous: 'Generous supporter',
};

function isKnownTier(value: string | null | undefined): value is TierKey {
  return value === 'coffee' || value === 'standard' || value === 'generous';
}

interface Props {
  status: Pick<
    SupporterStatusDto,
    'state' | 'name_plate' | 'current_tier_key'
  > | null;
  /** Compact (sm) vs full (md). Defaults to md. */
  size?: SupporterChipSize;
}

export function SupporterChip({ status, size = 'md' }: Props) {
  if (!status) return null;
  if (status.state !== 'active' && status.state !== 'lapsed') return null;

  // Unknown / null tier falls back to the standard palette + generic
  // "Supporter" label. Live data should always carry a tier when
  // state is active/lapsed, but the chip renders something useful
  // rather than nothing if the join row is missing.
  const tier: TierKey = isKnownTier(status.current_tier_key)
    ? status.current_tier_key
    : 'standard';
  const variant = status.state === 'active' ? 'active' : 'lapsed';
  const palette = TIER_PALETTES[tier][variant];
  const label =
    status.state === 'lapsed' ? `${TIER_LABELS[tier]} (lapsed)` : TIER_LABELS[tier];

  const padY = size === 'sm' ? 2 : 4;
  const padX = size === 'sm' ? 8 : 10;
  const fontSize = size === 'sm' ? 11 : 12;
  const gap = size === 'sm' ? 6 : 8;

  return (
    <span
      role="status"
      aria-label={status.name_plate ? `${label} — ${status.name_plate}` : label}
      className="mono"
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap,
        padding: `${padY}px ${padX}px`,
        fontSize,
        fontWeight: 500,
        lineHeight: 1.2,
        color: palette.fg,
        background: palette.bg,
        border: `1px solid ${palette.border}`,
        borderRadius: 'var(--r-pill)',
        whiteSpace: 'nowrap',
      }}
    >
      <span>{label}</span>
      {status.name_plate && (
        <span
          aria-hidden="true"
          style={{
            opacity: 0.55,
            fontWeight: 400,
            // The dot keeps the plate visually anchored to the label
            // without needing another border / divider element.
            // Reads cleanly in screen readers because the aria-label
            // on the parent already spells out the relationship.
          }}
        >
          · {status.name_plate}
        </span>
      )}
    </span>
  );
}
