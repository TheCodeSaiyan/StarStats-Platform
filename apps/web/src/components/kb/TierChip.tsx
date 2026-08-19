/**
 * Compact tier/subtype chip for location entities.
 *
 * Rendered adjacent to a location name to communicate its place in
 * the taxonomy at a glance: `Landmark · Drug lab`, `Landing zone ·
 * City`, `Space station`. The subtype slot is optional — bare-tier
 * entities (a system, an unclassified POI) show just the tier label.
 *
 * Why per-tier palette: the 8 tiers are mutually exclusive and span
 * a meaningful spectrum from "named city" to "procedural cave". A
 * subtle hue per tier makes the chip skim-readable in dense lists
 * (a journey timeline, a chain strip) without forcing the reader to
 * parse text.
 *
 * No `ss-badge` reuse: ss-badge forces uppercase + wide tracking,
 * which is right for status pills but mangles "Landing zone" /
 * "Drug lab" into "LANDING ZONE" / "DRUG LAB" — fine in isolation
 * but overpowering when stacked next to a location name. Inline
 * styles keep this contained to the chip surface.
 */

import React from 'react';
import {
  type LocationTier,
  type LocationSubtype,
  tierLabel,
  subtypeLabel,
} from '@/lib/reference-types';

/** Per-tier accent color. Returns a CSS color string (token reference
 *  or oklab mix) — not a class — so the chip can stay self-contained
 *  without polluting the global stylesheet. */
function tierAccent(tier: LocationTier): string {
  switch (tier) {
    case 'system':
      return 'var(--accent)';
    case 'astronomical_object':
      return 'var(--accent-2, var(--accent))';
    case 'landing_zone':
      return 'var(--ok)';
    case 'space_station':
      return 'var(--info, var(--accent))';
    case 'landmark':
      return 'var(--warn)';
    case 'flotilla':
      return 'var(--accent-3, var(--accent))';
    case 'naval_base':
      return 'var(--danger)';
    case 'anonymous_poi':
      return 'var(--fg-dim)';
  }
}

export function TierChip({
  tier,
  subtype,
  compact = false,
}: {
  tier: LocationTier;
  subtype?: LocationSubtype | string;
  /** When true, render only the subtype label (or tier label if no
   *  subtype). Used in contexts where the parent text already
   *  conveys the tier and a 2-token chip is too noisy. */
  compact?: boolean;
}) {
  const accent = tierAccent(tier);
  const primary = compact && subtype ? subtypeLabel(subtype) : tierLabel(tier);
  const secondary = !compact && subtype ? subtypeLabel(subtype) : null;

  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 4,
        padding: '1px 7px',
        borderRadius: 'var(--r-pill)',
        border: `1px solid color-mix(in oklab, ${accent} 40%, transparent)`,
        background: `color-mix(in oklab, ${accent} 10%, transparent)`,
        color: accent,
        fontSize: 10.5,
        lineHeight: 1.5,
        fontWeight: 500,
        letterSpacing: '0.01em',
        whiteSpace: 'nowrap',
        verticalAlign: 'baseline',
      }}
      aria-label={
        secondary ? `${primary} · ${secondary}` : primary
      }
    >
      <span>{primary}</span>
      {secondary && (
        <>
          <span aria-hidden style={{ opacity: 0.5 }}>·</span>
          <span style={{ color: 'var(--fg-muted)' }}>{secondary}</span>
        </>
      )}
    </span>
  );
}
