/**
 * Knowledge base landing skeleton. The landing fetches all four
 * category bundles (`loadAllReferenceBundles`) to derive entry
 * counts — the slowest KB fetch — so a scoped loading boundary here
 * gives instant feedback on navigation into `/kb` instead of a
 * frozen-looking click. Server Component — pure CSS pulse.
 */

export default function Loading() {
  return (
    <main aria-busy="true" aria-label="Loading knowledge base">
      <div className="skeleton" style={{ height: 32, width: 280, marginBottom: 20 }} />
      <hr className="ss-rule" style={{ margin: '20px 0 16px' }} />
      <div className="skeleton" style={{ height: 16, width: '80%', marginBottom: 8 }} />
      <div className="skeleton" style={{ height: 16, width: '55%' }} />
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
          gap: 16,
          marginTop: 24,
        }}
      >
        {Array.from({ length: 4 }).map((_, i) => (
          <div key={i} className="ss-card" style={{ padding: '20px 22px' }}>
            <div className="skeleton" style={{ height: 20, width: '50%', marginBottom: 10 }} />
            <div className="skeleton" style={{ height: 13, width: '90%', marginBottom: 6 }} />
            <div className="skeleton" style={{ height: 13, width: '70%', marginBottom: 18 }} />
            <div className="skeleton" style={{ height: 11, width: 90 }} />
          </div>
        ))}
      </div>
    </main>
  );
}
