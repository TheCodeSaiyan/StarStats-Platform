/**
 * Per-category browse skeleton. The category page fetches the full
 * category bundle server-side (`getCategoryBundle`) — large for
 * vehicles/items and `no-store`, so multi-second on a cold hit — and
 * has no client interactivity to paint until it resolves. Without a
 * scoped loading boundary the click looks dead until the server
 * component streams; this paints an instant list skeleton instead.
 * Server Component — pure CSS pulse.
 */

export default function Loading() {
  return (
    <div aria-busy="true" aria-label="Loading category">
      <div className="skeleton" style={{ height: 13, width: 140, marginBottom: 14 }} />
      <div className="skeleton" style={{ height: 32, width: 220 }} />
      <div className="skeleton" style={{ height: 13, width: 120, marginTop: 10 }} />

      {/* Search bar row */}
      <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
        <div className="skeleton" style={{ height: 36, flex: '1 1 280px' }} />
        <div className="skeleton" style={{ height: 36, width: 86 }} />
      </div>

      {/* Facet chip row */}
      <div style={{ display: 'flex', gap: 6, marginTop: 12, flexWrap: 'wrap' }}>
        {Array.from({ length: 6 }).map((_, i) => (
          <div key={i} className="skeleton" style={{ height: 22, width: 70 + (i % 3) * 24, borderRadius: 0 }} />
        ))}
      </div>

      {/* Card grid */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
          gap: 12,
          marginTop: 16,
        }}
      >
        {Array.from({ length: 12 }).map((_, i) => (
          <div key={i} className="ss-card" style={{ padding: '14px 16px' }}>
            <div className="skeleton" style={{ height: 15, width: '70%', marginBottom: 8 }} />
            <div className="skeleton" style={{ height: 11, width: '50%', marginBottom: 10 }} />
            <div className="skeleton" style={{ height: 11, width: '85%', marginBottom: 5 }} />
            <div className="skeleton" style={{ height: 11, width: '60%' }} />
          </div>
        ))}
      </div>
    </div>
  );
}
