/**
 * A single roadmap item on the list page. Server-component compatible
 * (no client hooks). Links into the detail page.
 */

import Link from 'next/link';
import type { Route } from 'next';
import type { RoadmapItemPublic } from '@/lib/roadmap';
import { StatusBadge } from './StatusBadge';
import { ChannelChipStrip } from './ChannelChipStrip';

const ETA_LABEL: Record<string, string> = {
  now: 'Now',
  next: 'Next',
  later: 'Later',
  someday: 'Someday',
  tbd: 'TBD',
};

export function RoadmapCard({ item }: { item: RoadmapItemPublic }) {
  return (
    <Link
      href={`/roadmap/${encodeURIComponent(item.slug)}` as Route}
      style={{
        textDecoration: 'none',
        color: 'inherit',
        display: 'block',
      }}
    >
      <article
        style={{
          padding: 20,
          borderRadius: 0,
          border: '1px solid var(--border)',
          background: 'var(--bg-elev, var(--bg))',
          marginBottom: 12,
        }}
      >
        <header
          style={{
            display: 'flex',
            alignItems: 'baseline',
            justifyContent: 'space-between',
            gap: 12,
            marginBottom: 8,
          }}
        >
          <h2
            style={{
              margin: 0,
              fontSize: 18,
              fontWeight: 600,
            }}
          >
            {item.title}
          </h2>
          <StatusBadge status={item.headline_status} />
        </header>

        {item.summary && (
          <p
            style={{
              margin: '6px 0 14px',
              color: 'var(--fg-dim)',
              fontSize: 14,
              lineHeight: 1.5,
            }}
          >
            {item.summary}
          </p>
        )}

        <ChannelChipStrip channels={item.channels} />

        <footer
          style={{
            marginTop: 14,
            display: 'flex',
            gap: 14,
            fontSize: 12,
            color: 'var(--fg-dim)',
            flexWrap: 'wrap',
          }}
        >
          {item.category && <span>{item.category}</span>}
          {item.eta_band && <span>· {ETA_LABEL[item.eta_band] ?? item.eta_band}</span>}
          <span>· {item.votes} vote{item.votes === 1 ? '' : 's'}</span>
        </footer>
      </article>
    </Link>
  );
}
