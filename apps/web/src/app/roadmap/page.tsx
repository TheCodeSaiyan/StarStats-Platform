import type { Metadata } from 'next';
import { listRoadmap } from '@/lib/roadmap';
import { RoadmapCard } from '@/components/roadmap/RoadmapCard';

export const metadata: Metadata = {
  title: 'Roadmap',
  description: 'What we are building, on which channel, and what just shipped.',
};

export default async function RoadmapPage() {
  const { items } = await listRoadmap();

  return (
    <main
      style={{
        maxWidth: 880,
        margin: '0 auto',
        padding: '48px 24px',
      }}
    >
      <header style={{ marginBottom: 32 }}>
        <span
          className="ss-placard"
          style={{ color: 'var(--fg-dim)' }}
        >
          01 · Roadmap
        </span>
        <h1
          style={{
            margin: '12px 0 0',
            fontSize: 'clamp(40px, 6vw, 64px)',
            fontWeight: 600,
          }}
        >
          What we&apos;re building
        </h1>
        <p
          style={{
            margin: '14px 0 0',
            color: 'var(--fg-dim)',
            maxWidth: 600,
            lineHeight: 1.55,
          }}
        >
          Tracked on a GitHub Project board and reflected here. Channel
          chips show where each feature sits in the release pipeline.
          Upvoting feeds back into prioritisation.
        </p>
      </header>

      {items.length === 0 ? (
        <p style={{ color: 'var(--fg-dim)', fontStyle: 'italic' }}>
          Nothing public yet.
        </p>
      ) : (
        items.map((item) => <RoadmapCard key={item.id} item={item} />)
      )}
    </main>
  );
}
