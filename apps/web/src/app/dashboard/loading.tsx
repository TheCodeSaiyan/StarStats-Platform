import { PageSkeleton } from '@/components/shell/PageSkeleton';

/**
 * Dashboard loading skeleton. Mirrors the redesigned Manifest layout —
 * eyebrow + h1 header, then the stat strip, heatmap card, and the
 * 2-col distribution + stream cards — so page-shift on hydration is
 * minimal. The skeleton itself is pure CSS animation; `PageSkeleton` wraps
 * it in the projection frame, which is a Client Component.
 */

export default function DashboardLoading() {
  return (
    <PageSkeleton label="Loading dashboard…">
      <header>
        <div
          className="skeleton"
          style={{ height: 12, width: 180, marginBottom: 12 }}
        />
        <div
          className="skeleton"
          style={{ height: 30, width: 260, marginBottom: 10 }}
        />
        <div className="skeleton" style={{ height: 14, width: 320 }} />
      </header>

      {/* Stat strip skeletons */}
      <div style={{ display: 'flex', gap: 12, flexWrap: 'nowrap' }}>
        {[0, 1, 2, 3].map((i) => (
          <div
            key={i}
            className="ss-card"
            style={{ flex: '1 1 200px', padding: '18px 20px', minWidth: 0 }}
          >
            <div
              className="skeleton"
              style={{ height: 11, width: 80, marginBottom: 10 }}
            />
            <div
              className="skeleton"
              style={{ height: 22, width: 100, marginBottom: 8 }}
            />
            <div className="skeleton" style={{ height: 11, width: 90 }} />
          </div>
        ))}
      </div>

      {/* Heatmap card skeleton */}
      <section className="ss-card" style={{ padding: '20px 24px 22px' }}>
        <div
          className="skeleton"
          style={{ height: 11, width: 60, marginBottom: 8 }}
        />
        <div
          className="skeleton"
          style={{ height: 16, width: 140, marginBottom: 18 }}
        />
        <div className="skeleton" style={{ height: 110, width: '100%' }} />
      </section>

      {/* Distribution + stream side-by-side */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '1fr 1.3fr',
          gap: 16,
        }}
      >
        <section className="ss-card" style={{ padding: '20px 24px 22px' }}>
          <div
            className="skeleton"
            style={{ height: 11, width: 80, marginBottom: 8 }}
          />
          <div
            className="skeleton"
            style={{ height: 16, width: 160, marginBottom: 18 }}
          />
          {[0, 1, 2, 3, 4].map((i) => (
            <div
              key={i}
              className="skeleton"
              style={{ height: 12, width: '100%', marginBottom: 12 }}
            />
          ))}
        </section>

        <section className="ss-card" style={{ padding: '20px 24px 22px' }}>
          <div
            className="skeleton"
            style={{ height: 11, width: 60, marginBottom: 8 }}
          />
          <div
            className="skeleton"
            style={{ height: 16, width: 180, marginBottom: 18 }}
          />
          {[0, 1, 2, 3, 4, 5].map((i) => (
            <div
              key={i}
              className="skeleton"
              style={{ height: 12, width: '92%', marginBottom: 10 }}
            />
          ))}
        </section>
      </div>
    </PageSkeleton>
  );
}
