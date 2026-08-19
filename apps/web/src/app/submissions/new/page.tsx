/**
 * `/submissions/new` — placeholder until the web-side submission form
 * lands. The canonical flow today is the desktop client's "Unknowns"
 * list (it's the only surface that has the raw log line + pattern in
 * hand at the moment of capture).
 *
 * Auth-gated like the rest of the submissions screens so a deep-link
 * still bounces unauthenticated visitors through `/auth/login`.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';

export const metadata = { title: "New submission" };

export default async function NewSubmissionPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/submissions/new');

  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Submissions · submit a new pattern
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 32,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          Coming soon
        </h1>
        <p
          style={{
            margin: '6px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
            maxWidth: 640,
          }}
        >
          For now, submit patterns from the desktop client&apos;s{' '}
          <span className="mono" style={{ color: 'var(--fg)' }}>
            Unknowns
          </span>{' '}
          list — that&apos;s the surface with the raw log line and the
          proposed parser already in hand. A web-side submission form is
          on the roadmap.
        </p>
      </header>

      <section className="ss-card" style={{ padding: '20px 24px' }}>
        <Link
          href={'/submissions' as Route}
          className="ss-btn ss-btn--ghost"
        >
          ← Back to submissions
        </Link>
      </section>
    </div>
  );
}
