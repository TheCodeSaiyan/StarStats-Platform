import type { Metadata } from 'next';
import { listChangelog } from '@/lib/roadmap';

export const metadata: Metadata = {
  title: 'Changelog',
  description: 'Recent feature releases and bug fixes per channel.',
};

const CHANNEL_LABEL: Record<string, string> = {
  live: 'Live',
  beta: 'Beta',
  alpha: 'Alpha',
  'tech-preview': 'Tech preview',
};

function fmtDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    });
  } catch {
    return iso;
  }
}

export default async function ChangelogPage() {
  const { entries } = await listChangelog();

  return (
    <main
      style={{
        maxWidth: 760,
        margin: '0 auto',
        padding: '48px 24px',
      }}
    >
      <header style={{ marginBottom: 32 }}>
        <span
          className="ss-placard"
          style={{ color: 'var(--fg-dim)' }}
        >
          Changelog
        </span>
        <h1
          style={{
            margin: '12px 0 0',
            fontSize: 'clamp(40px, 6vw, 64px)',
            fontWeight: 600,
          }}
        >
          What just shipped
        </h1>
      </header>

      {entries.length === 0 ? (
        <p style={{ color: 'var(--fg-dim)', fontStyle: 'italic' }}>
          No releases yet.
        </p>
      ) : (
        <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
          {entries.map((e) => (
            <li
              key={e.id}
              style={{
                borderTop: '1px solid var(--border)',
                padding: 'var(--s5) 0',
              }}
            >
              <header
                style={{
                  display: 'flex',
                  alignItems: 'baseline',
                  gap: 12,
                  flexWrap: 'wrap',
                  marginBottom: 8,
                }}
              >
                <h2
                  style={{
                    margin: 0,
                    fontSize: 'var(--fs-md)',
                    fontWeight: 600,
                  }}
                >
                  {e.title}
                </h2>
                <span
                  style={{
                    fontSize: 12,
                    color: 'var(--fg-dim)',
                  }}
                >
                  {CHANNEL_LABEL[e.channel] ?? e.channel} ·{' '}
                  {fmtDate(e.published_at)}
                </span>
              </header>
              <div
                style={{
                  whiteSpace: 'pre-wrap',
                  color: 'var(--fg)',
                  fontSize: 'var(--fs-base)',
                  lineHeight: 1.6,
                }}
              >
                {e.body}
              </div>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
