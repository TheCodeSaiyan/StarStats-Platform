import React from 'react';
import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { getAdminWaitlist, getWaitlistConfig, statusOf } from '@/lib/api';
import { logger } from '@/lib/logger';
import { WaitlistConsole } from './WaitlistConsole';

// No brand suffix: layout.tsx's title.template appends " — StarStats".
export const metadata = { title: 'Waitlist' };

export default async function AdminWaitlistPage() {
  const session = await getSession();
  // The admin layout already gates on staffRoles; this is here to narrow
  // the type for session.token, matching the other admin pages.
  if (!session) redirect('/auth/login?next=/admin/waitlist');

  // Settle independently per the multi-endpoint dashboard invariant: one
  // probe hiccup must not 500 the whole console.
  const [queuedRes, admittedRes, configRes] = await Promise.allSettled([
    getAdminWaitlist(session.token, { status: 'queued', limit: 200 }),
    getAdminWaitlist(session.token, { status: 'admitted', limit: 500 }),
    getWaitlistConfig(session.token),
  ]);

  if (queuedRes.status === 'rejected') {
    logger.warn(
      { err: queuedRes.reason, status: statusOf(queuedRes.reason) },
      'admin waitlist queue fetch failed',
    );
  }
  if (admittedRes.status === 'rejected') {
    logger.warn(
      { err: admittedRes.reason, status: statusOf(admittedRes.reason) },
      'admin waitlist admitted fetch failed',
    );
  }
  if (configRes.status === 'rejected') {
    logger.warn(
      { err: configRes.reason, status: statusOf(configRes.reason) },
      'admin waitlist config fetch failed',
    );
  }

  const queued = queuedRes.status === 'fulfilled' ? queuedRes.value : [];
  const admitted = admittedRes.status === 'fulfilled' ? admittedRes.value : [];
  const admittedCount = admitted.length;

  return (
    <main>

      <h1
        style={{
          margin: '0 0 var(--s2)',
          fontSize: 'clamp(28px, 4vw, 40px)',
          fontWeight: 600,
          letterSpacing: 'var(--tracking-tight)',
        }}
      >
        Waitlist
      </h1>
      <p style={{ color: 'var(--fg-muted)', marginTop: 0 }}>
        The public-beta signup gate: who is waiting, who is in, and the cap
        that decides.
      </p>

      {configRes.status === 'rejected' ? (
        // Never render a default cap here. A console showing "50" because
        // the read failed would be displaying a number that is not the one
        // enforcing admissions — the worst possible lie for this page.
        <p role="alert" className="ss-card" style={{ padding: 'var(--s5)' }}>
          Could not read the waitlist config, so the gate state and cap are
          unknown and cannot be changed from here. The queue below may also
          be stale. Check the API logs before assuming the gate is off.
        </p>
      ) : (
        <WaitlistConsole
          queued={queued}
          admitted={admitted}
          admittedCount={admittedCount}
          config={configRes.value}
        />
      )}
    </main>
  );
}
