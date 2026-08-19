/**
 * Generic loading skeleton for signed-in list/detail pages that fetch
 * on the server. Mirrors the header + stacked-card rhythm of the real
 * pages so the hand-off on hydration doesn't shift layout. Used by the
 * thin `loading.tsx` files for /journey, /sharing, /discover, /orgs —
 * pages that previously flashed a blank pane while their server fetch
 * resolved. Server Component; pure CSS animation via `.skeleton`.
 */
interface PageSkeletonProps {
  /** Accessible busy label, e.g. "Loading timeline…". */
  label?: string;
  /** Number of card placeholders to render. */
  cards?: number;
}

export function PageSkeleton({ label = 'Loading…', cards = 4 }: PageSkeletonProps) {
  return (
    <div
      aria-busy="true"
      aria-label={label}
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="skeleton" style={{ height: 12, width: 180, marginBottom: 12 }} />
        <div className="skeleton" style={{ height: 30, width: 260, marginBottom: 10 }} />
        <div className="skeleton" style={{ height: 14, width: 320 }} />
      </header>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        {Array.from({ length: cards }).map((_, i) => (
          <section key={i} className="ss-card" style={{ padding: '18px 20px' }}>
            <div className="skeleton" style={{ height: 12, width: '40%', marginBottom: 10 }} />
            <div className="skeleton" style={{ height: 12, width: '72%', marginBottom: 8 }} />
            <div className="skeleton" style={{ height: 12, width: '55%' }} />
          </section>
        ))}
      </div>
    </div>
  );
}
