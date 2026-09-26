import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata } from 'next';
import { listChangelog, listNews, listReleases, type NewsPost } from '@/lib/roadmap';
import { pairByDay } from '@/lib/releases';
import { ReleaseNotes } from '@/components/releases/ReleaseNotes';
import { logger } from '@/lib/logger';

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
  // Two feeds, settled separately: news failing must not blank the
  // release list, or the other way round.
  const [changelogRes, newsRes, releasesRes] = await Promise.allSettled([
    listChangelog(),
    listNews(10),
    listReleases(30),
  ]);
  if (releasesRes.status === 'rejected') {
    logger.warn({ err: releasesRes.reason, call: 'changelog.releases' }, 'releases fetch failed');
  }
  // One entry per day, pairing the tray and platform versions shipped that
  // day: players do not think in release tracks.
  const days = releasesRes.status === 'fulfilled' ? pairByDay(releasesRes.value.releases) : [];
  if (changelogRes.status === 'rejected') {
    logger.warn({ err: changelogRes.reason, call: 'changelog.list' }, 'changelog fetch failed');
  }
  if (newsRes.status === 'rejected') {
    logger.warn({ err: newsRes.reason, call: 'changelog.news' }, 'news fetch failed');
  }
  const entries = changelogRes.status === 'fulfilled' ? changelogRes.value.entries : [];
  const news: NewsPost[] = newsRes.status === 'fulfilled' ? newsRes.value.posts : [];

  return (
    <MarketingSurface
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Changelog' },
      ]}
      title="Changelog"
      ctx="What shipped, and when"
    >
      <DocsIndex active="/changelog" />
    <div
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

      {news.length > 0 ? (
        <section aria-labelledby="news-heading" style={{ marginBottom: 40 }}>
          <h2
            id="news-heading"
            style={{ margin: '0 0 8px', fontSize: 'var(--fs-lg)', fontWeight: 600 }}
          >
            News
          </h2>
          <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
            {news.map((n) => (
              <li
                key={n.id}
                data-testid="news-post"
                style={{ borderTop: '1px solid var(--border)', padding: 'var(--s4) 0' }}
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
                  <h3 style={{ margin: 0, fontSize: 'var(--fs-md)', fontWeight: 600 }}>
                    {n.title}
                  </h3>
                  {n.published_at ? (
                    <span style={{ fontSize: 12, color: 'var(--fg-dim)' }}>
                      {fmtDate(n.published_at)}
                    </span>
                  ) : null}
                </header>
                {/* Plain text, whitespace kept. Never rendered as HTML. */}
                <div style={{ whiteSpace: 'pre-wrap', lineHeight: 1.6 }}>{n.body}</div>
                {n.link_url ? (
                  <p style={{ margin: '8px 0 0' }}>
                    <a href={n.link_url} rel="noopener noreferrer" target="_blank">
                      Read more
                    </a>
                  </p>
                ) : null}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {days.length > 0 ? (
        <section aria-labelledby="releases-heading" style={{ marginBottom: 40 }}>
          <h2
            id="releases-heading"
            style={{ margin: '0 0 8px', fontSize: 'var(--fs-lg)', fontWeight: 600 }}
          >
            Releases
          </h2>
          <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
            {days.map((d) => (
              <li
                key={`${d.date}-${d.channel}`}
                data-testid="release-day"
                style={{ borderTop: '1px solid var(--border)', padding: 'var(--s5) 0' }}
              >
                <header style={{ display: 'flex', gap: 12, flexWrap: 'wrap', alignItems: 'baseline', marginBottom: 10 }}>
                  <h3 style={{ margin: 0, fontSize: 'var(--fs-md)', fontWeight: 600 }}>
                    {[d.tray && `Tray ${d.tray}`, d.platform && `Platform ${d.platform}`]
                      .filter(Boolean)
                      .join(' · ')}
                  </h3>
                  <span style={{ fontSize: 12, color: 'var(--fg-dim)' }}>{fmtDate(d.date)}</span>
                </header>
                <ReleaseNotes groups={d.groups} />
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {entries.length > 0 ? (
        <h2 style={{ margin: '0 0 8px', fontSize: 'var(--fs-lg)', fontWeight: 600 }}>
          Roadmap updates
        </h2>
      ) : null}
      {entries.length === 0 ? (
        days.length > 0 ? null : <p style={{ color: 'var(--fg-dim)', fontStyle: 'italic' }}>
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
    </div>
    </MarketingSurface>
  );
}
