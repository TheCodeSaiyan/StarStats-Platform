/**
 * Pill rendered next to events whose `metadata.source === 'inferred'`.
 *
 * Web mirror of `apps/tray-ui/src/timeline/InferredBadge.tsx`. The
 * tray pulls its visual tokens from `starstats-tokens.css` (shared
 * theme); this copy references the same CSS variables on the web
 * surface so themed renders stay consistent across apps.
 *
 * Why a duplicate component rather than a shared package: the tray
 * uses Vite + module-scoped styles and the web app uses Next.js with
 * RSC; they don't share a build target. Until the v2 metadata
 * pipeline stabilises (Phase 5+), the small duplication is cheaper
 * than a workspace package. See `the release design notesfollow-ups/` for
 * the tracking entry.
 *
 * Accessibility: the `title` attribute provides hover detail; the
 * mirroring `aria-label` makes the badge readable for screen-reader
 * users without needing the surrounding context.
 */

interface Props {
  confidence: number;
}

function clampConfidence(value: number): number {
  if (value < 0) return 0;
  if (value > 1) return 1;
  return value;
}

export function InferredBadge({ confidence }: Props) {
  const pct = Math.round(clampConfidence(confidence) * 100);
  const label = `Inferred event (confidence ${pct}%)`;
  return (
    <span
      role="status"
      aria-label={label}
      title={label}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '2px 8px',
        borderRadius: 'var(--r-pill)',
        background: 'var(--surface-2)',
        border: '1px solid var(--border)',
        color: 'var(--fg-muted)',
        fontSize: 10,
        fontWeight: 600,
        textTransform: 'uppercase',
        letterSpacing: '0.08em',
        fontFamily: 'var(--font-sans)',
      }}
    >
      <span>Inferred</span>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          color: 'var(--fg-dim)',
          letterSpacing: 0,
        }}
      >
        {pct}%
      </span>
    </span>
  );
}
