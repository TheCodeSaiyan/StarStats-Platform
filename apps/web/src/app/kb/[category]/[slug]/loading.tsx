import { PageSkeleton } from '@/components/shell/PageSkeleton';

/**
 * Entity detail skeleton. The detail page is the heaviest KB fetch:
 * the per-slug entry, the category `/stats` buckets, AND the full
 * category bundle for the comparison catalog (`no-store` for large
 * categories). A scoped loading boundary paints the hero + section
 * shells immediately so navigation into a ship/weapon/item/location
 * page never looks frozen. Server Component — pure CSS pulse.
 */

export default function Loading() {
  return (
    <PageSkeleton label="Loading entry…">
      <div className="skeleton" style={{ height: 13, width: 200 }} />

      {/* Hero */}
      <div className="ss-card" style={{ padding: '24px 24px 22px' }}>
        <div className="skeleton" style={{ height: 11, width: 80, marginBottom: 12 }} />
        <div className="skeleton" style={{ height: 36, width: '60%', marginBottom: 14 }} />
        <div style={{ display: 'flex', gap: 10 }}>
          <div className="skeleton" style={{ height: 20, width: 140 }} />
          <div className="skeleton" style={{ height: 20, width: 180 }} />
        </div>
      </div>

      {/* At a glance + two stat sections */}
      {Array.from({ length: 3 }).map((_, s) => (
        <div key={s} className="ss-card" style={{ padding: '18px 20px' }}>
          <div className="skeleton" style={{ height: 16, width: 160, marginBottom: 16 }} />
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))',
              gap: 18,
            }}
          >
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i}>
                <div className="skeleton" style={{ height: 11, width: '60%', marginBottom: 6 }} />
                <div className="skeleton" style={{ height: 16, width: '80%' }} />
              </div>
            ))}
          </div>
        </div>
      ))}
    </PageSkeleton>
  );
}
