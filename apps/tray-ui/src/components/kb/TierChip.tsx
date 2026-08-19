/**
 * Tray-side mirror of the web app's `<TierChip>` (apps/web/src/
 * components/kb/TierChip.tsx). Compact tier/subtype chip rendered
 * adjacent to a location name. Self-contained inline styles — the
 * tray and web have separate token sheets, but the variable names
 * (`--accent`, `--ok`, `--warn`, `--danger`, `--fg-dim`) overlap, so
 * the chip looks consistent across both shells without duplicating
 * a stylesheet.
 */

import {
  type LocationTier,
  type LocationSubtype,
  tierLabel,
  subtypeLabel,
} from '../../lib/reference';

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
        borderRadius: 'var(--r-pill, 999px)',
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
      // role="img" so the aria-label is honoured as a single accessible
      // name — on a bare <span> the label is announced inconsistently
      // across AT (L7). The inner primary/secondary spans stay visual.
      role="img"
      aria-label={secondary ? `${primary} · ${secondary}` : primary}
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
