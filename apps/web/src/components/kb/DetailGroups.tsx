import React from 'react';
import type { DetailGroup } from '@/lib/kb-detail';

/**
 * Renders the curated, grouped metadata sections (see `buildDetailGroups`)
 * as a stack of on-brand `.ss-card` sections — each a titled stat grid of
 * label/value pairs. Replaces the old flat metadata dump with a
 * scannable, organised presentation. Renders nothing when there are no
 * groups (so the page collapses cleanly for sparse entries).
 */
export function DetailGroups({ groups }: { groups: DetailGroup[] }) {
  if (groups.length === 0) return null;
  return (
    <>
      {groups.map((group) => (
        <section
          key={group.title}
          className="ss-card"
          style={{ padding: '18px 20px' }}
        >
          <h2
            style={{
              margin: '0 0 14px',
              fontSize: 14,
              fontWeight: 600,
              color: 'var(--fg)',
            }}
          >
            {group.title}
          </h2>
          <dl
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))',
              gap: 18,
              margin: 0,
            }}
          >
            {group.rows.map((row) => (
              <div key={row.label}>
                <dt
                  className="mono"
                  style={{
                    color: 'var(--fg-muted)',
                    fontSize: 11,
                    textTransform: 'uppercase',
                    letterSpacing: '0.08em',
                  }}
                >
                  {row.label}
                </dt>
                <dd
                  style={{
                    margin: '5px 0 0',
                    fontSize: 16,
                    color: 'var(--fg)',
                    lineHeight: 1.3,
                  }}
                >
                  {row.value}
                </dd>
              </div>
            ))}
          </dl>
        </section>
      ))}
    </>
  );
}
