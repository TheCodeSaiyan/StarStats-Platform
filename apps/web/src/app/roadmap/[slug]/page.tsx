import type { Metadata } from 'next';
import { notFound } from 'next/navigation';
import Link from 'next/link';
import type { Route } from 'next';
import { getRoadmapItem } from '@/lib/roadmap';
import { StatusBadge } from '@/components/roadmap/StatusBadge';
import { ChannelChipStrip } from '@/components/roadmap/ChannelChipStrip';

interface PageProps {
  params: Promise<{ slug: string }>;
}

export async function generateMetadata(
  { params }: PageProps,
): Promise<Metadata> {
  const { slug } = await params;
  const item = await getRoadmapItem(slug);
  if (!item) return { title: 'Not found · Roadmap' };
  return {
    // No brand suffix: layout.tsx's title.template appends " — StarStats".
    title: `${item.title} · Roadmap`,
    description: item.summary ?? undefined,
  };
}

export default async function RoadmapDetailPage({ params }: PageProps) {
  const { slug } = await params;
  const item = await getRoadmapItem(slug);
  if (!item) notFound();

  return (
    <main
      style={{
        maxWidth: 760,
        margin: '0 auto',
        padding: '48px 24px',
      }}
    >
      <Link
        href={'/roadmap' as Route}
        style={{
          color: 'var(--fg-dim)',
          fontSize: 'var(--fs-sm)',
          textDecoration: 'none',
        }}
      >
        ← Back to roadmap
      </Link>

      <header style={{ margin: '16px 0 24px' }}>
        <div
          style={{
            display: 'flex',
            alignItems: 'baseline',
            justifyContent: 'space-between',
            gap: 12,
          }}
        >
          <h1
            style={{
              margin: 0,
              fontSize: 'clamp(24px, 3vw, 32px)',
              fontWeight: 600,
            }}
          >
            {item.title}
          </h1>
          <StatusBadge status={item.headline_status} />
        </div>

        {item.summary && (
          <p
            style={{
              margin: '14px 0 0',
              color: 'var(--fg-dim)',
              lineHeight: 1.6,
            }}
          >
            {item.summary}
          </p>
        )}
      </header>

      <section style={{ marginBottom: 32 }}>
        <h2
          style={{
            fontSize: 14,
            fontWeight: 600,
            color: 'var(--fg-dim)',
            textTransform: 'uppercase',
            letterSpacing: 1,
            marginBottom: 12,
          }}
        >
          Channel status
        </h2>
        <ChannelChipStrip channels={item.channels} detailed />
      </section>

      <section style={{ display: 'flex', gap: 24, flexWrap: 'wrap' }}>
        {item.category && (
          <MetaField label="Category" value={item.category} />
        )}
        {item.eta_band && (
          <MetaField label="ETA band" value={item.eta_band} />
        )}
        <MetaField label="Votes" value={String(item.votes)} />
      </section>
    </main>
  );
}

function MetaField({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div
        className="ss-placard"
        style={{ color: 'var(--fg-dim)', marginBottom: 'var(--s1)' }}
      >
        {label}
      </div>
      <div style={{ fontSize: 14 }}>{value}</div>
    </div>
  );
}
